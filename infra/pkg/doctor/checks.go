/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package doctor

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"strings"
	"time"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/dynamic"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
)

// checker holds shared state for running all checks.
type checker struct {
	cs         *kubernetes.Clientset
	restConfig *rest.Config
	cfg        Config
	values     HelmValues
}

// checkPodReady checks if at least one pod matching the label selector is Running
// with all containers ready.
func (c *checker) checkPodReady(ctx context.Context, def PodCheckDef) CheckResult {
	pods, err := c.cs.CoreV1().Pods(c.cfg.Namespace).List(ctx, metav1.ListOptions{
		LabelSelector: def.LabelSelector,
		Limit:         1,
	})
	if err != nil {
		return CheckResult{Name: def.Name, Status: StatusFail, Detail: fmt.Sprintf("list pods: %v", err)}
	}
	if len(pods.Items) == 0 {
		return CheckResult{Name: def.Name, Status: StatusSkip, Detail: "no pods found"}
	}

	pod := pods.Items[0]
	total := len(pod.Status.ContainerStatuses)
	ready := 0
	for _, cs := range pod.Status.ContainerStatuses {
		if cs.Ready {
			ready++
		}
	}

	detail := fmt.Sprintf("%d/%d %s", ready, total, pod.Status.Phase)
	if pod.Status.Phase == corev1.PodRunning && ready == total && total > 0 {
		return CheckResult{Name: def.Name, Status: StatusPass, Detail: detail}
	}
	return CheckResult{Name: def.Name, Status: StatusFail, Detail: detail}
}

// checkHelmRelease verifies that the Helm release has a deployed secret.
func (c *checker) checkHelmRelease(ctx context.Context) CheckResult {
	secrets, err := c.cs.CoreV1().Secrets(c.cfg.Namespace).List(ctx, metav1.ListOptions{
		LabelSelector: fmt.Sprintf("owner=helm,name=%s,status=deployed", c.cfg.Release),
	})
	if err != nil {
		return CheckResult{Name: "release deployed", Status: StatusFail, Detail: fmt.Sprintf("list secrets: %v", err)}
	}
	if len(secrets.Items) == 0 {
		return CheckResult{Name: "release deployed", Status: StatusFail, Detail: "no deployed release found"}
	}
	return CheckResult{Name: "release deployed", Status: StatusPass, Detail: "deployed"}
}

// checkServiceEndpoint checks a service via kube-apiserver proxy.
func (c *checker) checkServiceEndpoint(ctx context.Context, def EndpointCheckDef) CheckResult {
	// Check if service exists first.
	_, err := c.cs.CoreV1().Services(c.cfg.Namespace).Get(ctx, def.ServiceName, metav1.GetOptions{})
	if err != nil {
		return CheckResult{Name: def.Name, Status: StatusSkip, Detail: "service not found"}
	}

	// Proxy request through kube-apiserver.
	path := fmt.Sprintf("/api/v1/namespaces/%s/services/http:%s:%s/proxy%s",
		c.cfg.Namespace, def.ServiceName, def.Port, def.Path)
	result := c.cs.CoreV1().RESTClient().Get().AbsPath(path).Do(ctx)
	if result.Error() != nil {
		return CheckResult{Name: def.Name, Status: StatusFail, Detail: "service proxy request failed"}
	}
	return CheckResult{Name: def.Name, Status: StatusPass, Detail: "proxied via kube-apiserver"}
}

// checkTLSCertificate checks the wildcard certificate status via cert-manager CRD.
func (c *checker) checkTLSCertificate(ctx context.Context) CheckResult {
	dynClient, err := dynamic.NewForConfig(c.restConfig)
	if err != nil {
		return CheckResult{Name: "wildcard certificate", Status: StatusFail, Detail: fmt.Sprintf("dynamic client: %v", err)}
	}

	certGVR := schema.GroupVersionResource{
		Group:    "cert-manager.io",
		Version:  "v1",
		Resource: "certificates",
	}

	certs, err := dynClient.Resource(certGVR).Namespace(c.cfg.Namespace).List(ctx, metav1.ListOptions{})
	if err != nil {
		return CheckResult{Name: "wildcard certificate", Status: StatusFail, Detail: fmt.Sprintf("list certificates: %v", err)}
	}

	wildcardDNS := "*." + c.cfg.Domain
	for _, cert := range certs.Items {
		spec, ok := cert.Object["spec"].(map[string]interface{})
		if !ok {
			continue
		}
		dnsNames, ok := spec["dnsNames"].([]interface{})
		if !ok {
			continue
		}
		for _, dns := range dnsNames {
			if fmt.Sprintf("%v", dns) != wildcardDNS {
				continue
			}
			// Found the wildcard cert. Check Ready condition.
			status, ok := cert.Object["status"].(map[string]interface{})
			if !ok {
				return CheckResult{Name: "wildcard certificate", Status: StatusFail, Detail: "no status"}
			}
			conditions, ok := status["conditions"].([]interface{})
			if !ok {
				return CheckResult{Name: "wildcard certificate", Status: StatusFail, Detail: "no conditions"}
			}
			for _, cond := range conditions {
				cm, ok := cond.(map[string]interface{})
				if !ok {
					continue
				}
				if fmt.Sprintf("%v", cm["type"]) == "Ready" {
					if fmt.Sprintf("%v", cm["status"]) == "True" {
						return CheckResult{Name: "wildcard certificate", Status: StatusPass, Detail: "Ready"}
					}
					return CheckResult{Name: "wildcard certificate", Status: StatusFail, Detail: "Not Ready"}
				}
			}
			return CheckResult{Name: "wildcard certificate", Status: StatusFail, Detail: "Ready condition not found"}
		}
	}

	return CheckResult{Name: "wildcard certificate", Status: StatusSkip, Detail: "no wildcard certificate found"}
}

// checkConsulKV counts Consul KV keys under rara/config/ via kube-apiserver service proxy.
func (c *checker) checkConsulKV(ctx context.Context) CheckResult {
	path := fmt.Sprintf("/api/v1/namespaces/%s/services/http:consul-server:8500/proxy/v1/kv/rara/config/",
		c.cfg.Namespace)

	raw, err := c.cs.CoreV1().RESTClient().Get().
		AbsPath(path).
		Param("keys", "").
		DoRaw(ctx)
	if err != nil {
		return CheckResult{Name: "Consul KV (rara/config/)", Status: StatusFail, Detail: "no keys found"}
	}

	var keys []string
	if err := json.Unmarshal(raw, &keys); err != nil {
		return CheckResult{Name: "Consul KV (rara/config/)", Status: StatusFail, Detail: fmt.Sprintf("parse keys: %v", err)}
	}

	if len(keys) == 0 {
		return CheckResult{Name: "Consul KV (rara/config/)", Status: StatusFail, Detail: "no keys found"}
	}

	return CheckResult{
		Name:   "Consul KV (rara/config/)",
		Status: StatusPass,
		Detail: fmt.Sprintf("%d keys", len(keys)),
	}
}

// checkPVCs counts PVCs and their bound status.
func (c *checker) checkPVCs(ctx context.Context) CheckResult {
	pvcs, err := c.cs.CoreV1().PersistentVolumeClaims(c.cfg.Namespace).List(ctx, metav1.ListOptions{})
	if err != nil {
		return CheckResult{Name: "PVC status", Status: StatusFail, Detail: fmt.Sprintf("list PVCs: %v", err)}
	}

	total := len(pvcs.Items)
	bound := 0
	for _, pvc := range pvcs.Items {
		if pvc.Status.Phase == corev1.ClaimBound {
			bound++
		}
	}

	detail := fmt.Sprintf("%d/%d Bound", bound, total)
	if bound == total && total > 0 {
		return CheckResult{Name: "PVC status", Status: StatusPass, Detail: detail}
	}
	if total == 0 {
		return CheckResult{Name: "PVC status", Status: StatusSkip, Detail: "no PVCs found"}
	}
	return CheckResult{Name: "PVC status", Status: StatusWarn, Detail: detail}
}

// getTraefikLBIP retrieves the LoadBalancer IP from the Traefik service.
func (c *checker) getTraefikLBIP(ctx context.Context) (string, error) {
	svc, err := c.cs.CoreV1().Services(c.cfg.Namespace).Get(ctx, c.cfg.Release+"-traefik", metav1.GetOptions{})
	if err != nil {
		return "", fmt.Errorf("get traefik service: %w", err)
	}

	ingress := svc.Status.LoadBalancer.Ingress
	if len(ingress) == 0 {
		return "", fmt.Errorf("no LoadBalancer ingress found")
	}

	ip := ingress[0].IP
	if ip == "" {
		ip = ingress[0].Hostname
	}
	if ip == "" {
		return "", fmt.Errorf("LoadBalancer IP/hostname is empty")
	}

	return ip, nil
}

// checkHTTPSEndpoint checks an HTTPS ingress endpoint using Go's HTTP client.
func (c *checker) checkHTTPSEndpoint(ctx context.Context, def IngressCheckDef, lbIP string) CheckResult {
	host := def.Subdomain + "." + c.cfg.Domain
	url := fmt.Sprintf("https://%s%s", host, def.Path)

	if lbIP == "pending" || lbIP == "" {
		return CheckResult{Name: def.Subdomain, Status: StatusSkip, Detail: url + "  (no LB IP)"}
	}

	transport := &http.Transport{
		TLSClientConfig: &tls.Config{InsecureSkipVerify: true},
		DialContext:     dialWithResolve(lbIP, c.cfg.Domain),
	}

	client := &http.Client{
		Timeout:   5 * time.Second,
		Transport: transport,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}

	req, err := http.NewRequestWithContext(ctx, "GET", url, nil)
	if err != nil {
		return CheckResult{Name: def.Subdomain, Status: StatusFail, Detail: fmt.Sprintf("build request: %v", err)}
	}

	resp, err := client.Do(req)
	if err != nil {
		return CheckResult{Name: def.Subdomain, Status: StatusFail, Detail: fmt.Sprintf("%s  (HTTP 000)", url)}
	}
	defer resp.Body.Close()

	detail := fmt.Sprintf("%s  (HTTP %d)", url, resp.StatusCode)
	if resp.StatusCode >= 200 && resp.StatusCode < 400 {
		return CheckResult{Name: def.Subdomain, Status: StatusPass, Detail: detail}
	}
	return CheckResult{Name: def.Subdomain, Status: StatusFail, Detail: detail}
}

// dialWithResolve returns a DialContext function that resolves *.domain to the given LB IP.
func dialWithResolve(lbIP, domain string) func(ctx context.Context, network, addr string) (net.Conn, error) {
	return func(ctx context.Context, network, addr string) (net.Conn, error) {
		host, port, err := net.SplitHostPort(addr)
		if err != nil {
			return nil, err
		}
		if strings.HasSuffix(host, "."+domain) || host == domain {
			addr = net.JoinHostPort(lbIP, port)
		}
		return (&net.Dialer{Timeout: 5 * time.Second}).DialContext(ctx, network, addr)
	}
}
