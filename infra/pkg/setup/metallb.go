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

package setup

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"os/exec"
	"strings"
	"time"

	"k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/util/wait"
	k8syaml "k8s.io/apimachinery/pkg/util/yaml"
	"k8s.io/client-go/dynamic"
	"k8s.io/client-go/rest"
)

const (
	metallbVersion     = "v0.14.9"
	metallbManifestURL = "https://raw.githubusercontent.com/metallb/metallb/" + metallbVersion + "/config/manifests/metallb-native.yaml"
	metallbNamespace   = "metallb-system"
)

// InstallMetalLB installs MetalLB and configures an IP pool from the kind Docker network.
func InstallMetalLB(ctx context.Context, rc *rest.Config) error {
	if err := Wait("Applying MetalLB manifests", func() error {
		return applyMetalLBManifests(ctx, rc)
	}); err != nil {
		return err
	}

	if err := Wait("Waiting for MetalLB controller", func() error {
		return waitForMetalLBController(ctx, rc)
	}); err != nil {
		return err
	}

	subnet, err := getKindDockerSubnet()
	if err != nil {
		return fmt.Errorf("get kind docker subnet: %w", err)
	}
	Info(fmt.Sprintf("kind Docker network subnet: %s", subnet))

	startIP, endIP, err := computeMetalLBRange(subnet)
	if err != nil {
		return fmt.Errorf("compute metallb range: %w", err)
	}
	Info(fmt.Sprintf("MetalLB IP pool: %s - %s", startIP, endIP))

	if err := Wait("Configuring MetalLB IP pool", func() error {
		return applyMetalLBConfig(ctx, rc, startIP, endIP)
	}); err != nil {
		return err
	}

	return nil
}

// applyMetalLBManifests fetches and applies the MetalLB manifests.
func applyMetalLBManifests(ctx context.Context, rc *rest.Config) error {
	resp, err := http.Get(metallbManifestURL)
	if err != nil {
		return fmt.Errorf("fetch metallb manifests: %w", err)
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("read metallb manifests: %w", err)
	}

	return applyMultiDocYAML(ctx, rc, body)
}

// getKindDockerSubnet returns the IPv4 subnet of the kind Docker network.
func getKindDockerSubnet() (string, error) {
	out, err := exec.Command("docker", "network", "inspect", "kind",
		"--format", "{{range .IPAM.Config}}{{.Subnet}}{{end}}").Output()
	if err != nil {
		return "", fmt.Errorf("docker network inspect: %w", err)
	}

	subnets := strings.Fields(strings.TrimSpace(string(out)))
	for _, s := range subnets {
		if !strings.Contains(s, ":") { // skip IPv6
			return s, nil
		}
	}
	return "", fmt.Errorf("no IPv4 subnet found in kind network")
}

// computeMetalLBRange returns start/end IPs from the upper portion of the subnet.
func computeMetalLBRange(cidr string) (string, string, error) {
	_, ipNet, err := net.ParseCIDR(cidr)
	if err != nil {
		return "", "", fmt.Errorf("parse cidr %q: %w", cidr, err)
	}

	base := ipNet.IP.To4()
	if base == nil {
		return "", "", fmt.Errorf("not an IPv4 network: %s", cidr)
	}

	start := make(net.IP, 4)
	end := make(net.IP, 4)
	copy(start, base)
	copy(end, base)

	_, bits := ipNet.Mask.Size()
	ones, _ := ipNet.Mask.Size()
	hostBits := bits - ones

	start[3] = 200
	end[3] = 250
	if hostBits >= 16 {
		start[2] = 255
		end[2] = 255
	}

	return start.String(), end.String(), nil
}

// applyMetalLBConfig creates the IPAddressPool and L2Advertisement CRs.
func applyMetalLBConfig(ctx context.Context, rc *rest.Config, startIP, endIP string) error {
	dc, err := dynamic.NewForConfig(rc)
	if err != nil {
		return err
	}

	poolGVR := schema.GroupVersionResource{Group: "metallb.io", Version: "v1beta1", Resource: "ipaddresspools"}
	l2GVR := schema.GroupVersionResource{Group: "metallb.io", Version: "v1beta1", Resource: "l2advertisements"}

	pool := &unstructured.Unstructured{
		Object: map[string]interface{}{
			"apiVersion": "metallb.io/v1beta1",
			"kind":       "IPAddressPool",
			"metadata": map[string]interface{}{
				"name":      "rara-pool",
				"namespace": metallbNamespace,
			},
			"spec": map[string]interface{}{
				"addresses": []interface{}{
					fmt.Sprintf("%s-%s", startIP, endIP),
				},
			},
		},
	}

	l2adv := &unstructured.Unstructured{
		Object: map[string]interface{}{
			"apiVersion": "metallb.io/v1beta1",
			"kind":       "L2Advertisement",
			"metadata": map[string]interface{}{
				"name":      "rara-l2adv",
				"namespace": metallbNamespace,
			},
			"spec": map[string]interface{}{
				"ipAddressPools": []interface{}{"rara-pool"},
			},
		},
	}

	retryCtx, cancel := context.WithTimeout(ctx, 3*time.Minute)
	defer cancel()

	for _, obj := range []*unstructured.Unstructured{pool, l2adv} {
		var gvr schema.GroupVersionResource
		switch obj.GetKind() {
		case "IPAddressPool":
			gvr = poolGVR
		case "L2Advertisement":
			gvr = l2GVR
		}

		localObj := obj
		if err := wait.PollUntilContextTimeout(retryCtx, 3*time.Second, 3*time.Minute, true, func(ctx context.Context) (bool, error) {
			_, applyErr := dc.Resource(gvr).Namespace(metallbNamespace).Apply(ctx,
				localObj.GetName(), localObj,
				metav1.ApplyOptions{FieldManager: "rara-setup", Force: true},
			)
			if applyErr != nil {
				if errors.IsNotFound(applyErr) || strings.Contains(applyErr.Error(), "no kind") {
					return false, nil // CRD not ready yet
				}
				return false, applyErr
			}
			return true, nil
		}); err != nil {
			return fmt.Errorf("apply %s: %w", obj.GetKind(), err)
		}
	}

	return nil
}

// waitForMetalLBController waits until the MetalLB controller deployment has available replicas.
func waitForMetalLBController(ctx context.Context, rc *rest.Config) error {
	dc, err := dynamic.NewForConfig(rc)
	if err != nil {
		return err
	}

	deployGVR := schema.GroupVersionResource{Group: "apps", Version: "v1", Resource: "deployments"}

	return wait.PollUntilContextTimeout(ctx, 5*time.Second, 5*time.Minute, true, func(ctx context.Context) (bool, error) {
		obj, err := dc.Resource(deployGVR).Namespace(metallbNamespace).Get(ctx, "controller", metav1.GetOptions{})
		if err != nil {
			return false, nil
		}
		availableReplicas, _, _ := unstructured.NestedInt64(obj.Object, "status", "availableReplicas")
		return availableReplicas > 0, nil
	})
}

// applyMultiDocYAML applies a multi-document YAML to the cluster using server-side apply.
func applyMultiDocYAML(ctx context.Context, rc *rest.Config, data []byte) error {
	dc, err := dynamic.NewForConfig(rc)
	if err != nil {
		return err
	}

	decoder := k8syaml.NewYAMLOrJSONDecoder(bytes.NewReader(data), 4096)
	for {
		var rawObj map[string]interface{}
		if err := decoder.Decode(&rawObj); err != nil {
			if err == io.EOF {
				break
			}
			return fmt.Errorf("decode yaml: %w", err)
		}
		if rawObj == nil {
			continue
		}

		obj := &unstructured.Unstructured{Object: rawObj}
		if obj.GetName() == "" {
			continue
		}

		gvr, err := gvrFromObject(obj)
		if err != nil {
			// Skip resources we can't map
			continue
		}

		ns := obj.GetNamespace()
		if ns != "" {
			if _, err := dc.Resource(gvr).Namespace(ns).Apply(ctx, obj.GetName(), obj,
				metav1.ApplyOptions{FieldManager: "rara-setup", Force: true}); err != nil {
				// Ignore failures on individual resources during initial apply
				continue
			}
		} else {
			if _, err := dc.Resource(gvr).Apply(ctx, obj.GetName(), obj,
				metav1.ApplyOptions{FieldManager: "rara-setup", Force: true}); err != nil {
				continue
			}
		}
	}
	return nil
}

// gvrFromObject returns the GVR for a given unstructured object using a simple heuristic.
func gvrFromObject(obj *unstructured.Unstructured) (schema.GroupVersionResource, error) {
	gv, err := schema.ParseGroupVersion(obj.GetAPIVersion())
	if err != nil {
		return schema.GroupVersionResource{}, err
	}

	kind := obj.GetKind()
	resource := strings.ToLower(kind)
	switch {
	case strings.HasSuffix(resource, "s"):
		// already plural
	case strings.HasSuffix(resource, "y"):
		resource = resource[:len(resource)-1] + "ies"
	default:
		resource += "s"
	}

	return schema.GroupVersionResource{
		Group:    gv.Group,
		Version:  gv.Version,
		Resource: resource,
	}, nil
}
