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

// Package setup provides a complete kind-based local development environment
// for rara. It creates a kind cluster, installs MetalLB for LoadBalancer support,
// deploys all infrastructure Helm charts, deploys custom K8s services, seeds
// Consul KV, and manages /etc/hosts entries.
package setup

import (
	"context"
	"fmt"
	"time"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/util/wait"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
)

const totalSteps = 8

// consolePrint is a Sender that routes events to the legacy console functions.
var consolePrint Sender = func(ev ProgressEvent) {
	switch ev.Kind {
	case EventInfo:
		Info(ev.Name)
	case EventWarn:
		Warn(ev.Name)
	}
}

// Up brings up the complete local rara environment.
func Up(ctx context.Context, cfg Config, send Sender) error {
	// Step 1: kind cluster
	const step1Name = "Ensure kind cluster"
	step1Start := time.Now()
	send(ProgressEvent{Kind: EventStepStart, N: 1, Total: totalSteps, Name: step1Name})
	kubeconfigPath, err := EnsureCluster(ctx, cfg, send)
	if err != nil {
		send(ProgressEvent{Kind: EventError, Err: fmt.Errorf("ensure cluster: %w", err)})
		return fmt.Errorf("ensure cluster: %w", err)
	}
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("kubeconfig: %s", kubeconfigPath)})
	send(ProgressEvent{Kind: EventStepDone, N: 1, Total: totalSteps, Name: step1Name, Elapsed: time.Since(step1Start)})

	// Step 2: MetalLB
	const step2Name = "Install MetalLB (LoadBalancer support)"
	step2Start := time.Now()
	send(ProgressEvent{Kind: EventStepStart, N: 2, Total: totalSteps, Name: step2Name})
	rc, err := clientcmd.BuildConfigFromFlags("", kubeconfigPath)
	if err != nil {
		send(ProgressEvent{Kind: EventError, Err: fmt.Errorf("build rest config: %w", err)})
		return fmt.Errorf("build rest config: %w", err)
	}
	if err := InstallMetalLB(ctx, rc, send); err != nil {
		send(ProgressEvent{Kind: EventError, Err: fmt.Errorf("install metallb: %w", err)})
		return fmt.Errorf("install metallb: %w", err)
	}
	send(ProgressEvent{Kind: EventStepDone, N: 2, Total: totalSteps, Name: step2Name, Elapsed: time.Since(step2Start)})

	// Step 3: Helm charts (infra stack)
	const step3Name = "Install infrastructure Helm charts"
	step3Start := time.Now()
	send(ProgressEvent{Kind: EventStepStart, N: 3, Total: totalSteps, Name: step3Name})
	if err := InstallHelmCharts(ctx, cfg, kubeconfigPath, send); err != nil {
		send(ProgressEvent{Kind: EventError, Err: fmt.Errorf("install helm charts: %w", err)})
		return fmt.Errorf("install helm charts: %w", err)
	}
	send(ProgressEvent{Kind: EventStepDone, N: 3, Total: totalSteps, Name: step3Name, Elapsed: time.Since(step3Start)})

	// Step 4: Custom K8s services
	const step4Name = "Deploy custom K8s services"
	step4Start := time.Now()
	send(ProgressEvent{Kind: EventStepStart, N: 4, Total: totalSteps, Name: step4Name})
	if err := DeployCustomServices(ctx, cfg, kubeconfigPath, send); err != nil {
		send(ProgressEvent{Kind: EventError, Err: fmt.Errorf("deploy custom services: %w", err)})
		return fmt.Errorf("deploy custom services: %w", err)
	}
	send(ProgressEvent{Kind: EventStepDone, N: 4, Total: totalSteps, Name: step4Name, Elapsed: time.Since(step4Start)})

	// Step 5: Seed Consul KV
	const step5Name = "Seed Consul KV"
	step5Start := time.Now()
	send(ProgressEvent{Kind: EventStepStart, N: 5, Total: totalSteps, Name: step5Name})
	if err := SeedConsulKV(ctx, cfg, kubeconfigPath, send); err != nil {
		send(ProgressEvent{Kind: EventError, Err: fmt.Errorf("seed consul kv: %w", err)})
		return fmt.Errorf("seed consul kv: %w", err)
	}
	send(ProgressEvent{Kind: EventStepDone, N: 5, Total: totalSteps, Name: step5Name, Elapsed: time.Since(step5Start)})

	// Step 6: Get LoadBalancer IP (Traefik)
	const step6Name = "Discover Traefik LoadBalancer IP"
	step6Start := time.Now()
	send(ProgressEvent{Kind: EventStepStart, N: 6, Total: totalSteps, Name: step6Name})
	lbIP, err := GetTraefikIP(ctx, rc, cfg)
	if err != nil {
		send(ProgressEvent{Kind: EventWarn, Name: fmt.Sprintf("Could not determine Traefik IP: %v", err)})
		send(ProgressEvent{Kind: EventWarn, Name: "You may need to update /etc/hosts manually"})
		lbIP = ""
	} else {
		send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Traefik LoadBalancer IP: %s", lbIP)})
	}
	send(ProgressEvent{Kind: EventStepDone, N: 6, Total: totalSteps, Name: step6Name, Elapsed: time.Since(step6Start)})

	// Step 7: /etc/hosts
	const step7Name = "Update /etc/hosts"
	step7Start := time.Now()
	send(ProgressEvent{Kind: EventStepStart, N: 7, Total: totalSteps, Name: step7Name})
	if lbIP != "" {
		send(ProgressEvent{Kind: EventInfo, Name: "Writing /etc/hosts entries..."})
		if err := AddHostsEntries(lbIP, cfg.Domain); err != nil {
			send(ProgressEvent{Kind: EventWarn, Name: fmt.Sprintf("Could not update /etc/hosts (try running with sudo): %v", err)})
		} else {
			send(ProgressEvent{Kind: EventInfo, Name: "Updated /etc/hosts"})
		}
	} else {
		send(ProgressEvent{Kind: EventInfo, Name: "Skipping /etc/hosts (no LoadBalancer IP)"})
	}
	send(ProgressEvent{Kind: EventStepDone, N: 7, Total: totalSteps, Name: step7Name, Elapsed: time.Since(step7Start)})

	// Step 8: Summary
	const step8Name = "Setup complete"
	step8Start := time.Now()
	send(ProgressEvent{Kind: EventStepStart, N: 8, Total: totalSteps, Name: step8Name})
	printSummary(cfg, lbIP)
	send(ProgressEvent{Kind: EventStepDone, N: 8, Total: totalSteps, Name: step8Name, Elapsed: time.Since(step8Start)})
	send(ProgressEvent{Kind: EventDone})

	return nil
}

// Down tears down the kind cluster and removes /etc/hosts entries.
func Down(cfg Config) error {
	fmt.Printf("\033[1;31m==> rara local teardown\033[0m\n\n")

	Step(1, 2, "Remove /etc/hosts entries")
	if err := Wait("Removing /etc/hosts entries", func() error {
		return RemoveHostsEntries()
	}); err != nil {
		Warn(fmt.Sprintf("Could not remove /etc/hosts entries: %v", err))
	}

	Step(2, 2, "Delete kind cluster")
	if err := Wait(fmt.Sprintf("Deleting kind cluster %q", cfg.ClusterName), func() error {
		return DeleteCluster(cfg.ClusterName, consolePrint)
	}); err != nil {
		return fmt.Errorf("delete cluster: %w", err)
	}

	OK("Teardown complete")
	return nil
}

// Status prints the status of the local environment.
func Status(ctx context.Context, cfg Config) error {
	kubeconfigPath := KindKubeconfigPath(cfg.ClusterName)

	exists, err := ClusterExists(cfg.ClusterName)
	if err != nil {
		return err
	}
	if !exists {
		fmt.Printf("Cluster %q does not exist\n", cfg.ClusterName)
		return nil
	}

	fmt.Printf("\033[1;32mCluster:\033[0m %s (exists)\n", cfg.ClusterName)
	fmt.Printf("Kubeconfig: %s\n\n", kubeconfigPath)

	rc, err := clientcmd.BuildConfigFromFlags("", kubeconfigPath)
	if err != nil {
		return fmt.Errorf("build rest config: %w", err)
	}

	lbIP, err := GetTraefikIP(ctx, rc, cfg)
	if err != nil {
		fmt.Printf("Traefik LoadBalancer IP: unknown (%v)\n", err)
		lbIP = ""
	} else {
		fmt.Printf("Traefik LoadBalancer IP: %s\n", lbIP)
	}

	printSummary(cfg, lbIP)
	return nil
}

// GetTraefikIP returns the LoadBalancer IP assigned to the Traefik service.
func GetTraefikIP(ctx context.Context, rc *rest.Config, cfg Config) (string, error) {
	return getLoadBalancerIP(ctx, rc, cfg.Namespace, fmt.Sprintf("%s-traefik", cfg.Prefix()))
}

// getLoadBalancerIP returns the first LoadBalancer ingress IP for a service,
// waiting up to 2 minutes for one to be assigned.
func getLoadBalancerIP(ctx context.Context, rc *rest.Config, ns, svcName string) (string, error) {
	kc, err := kubernetes.NewForConfig(rc)
	if err != nil {
		return "", err
	}

	var ip string
	err = wait.PollUntilContextTimeout(ctx, 5*time.Second, 2*time.Minute, true, func(ctx context.Context) (bool, error) {
		svc, err := kc.CoreV1().Services(ns).Get(ctx, svcName, metav1.GetOptions{})
		if err != nil {
			return false, nil
		}
		for _, ingress := range svc.Status.LoadBalancer.Ingress {
			if ingress.IP != "" {
				ip = ingress.IP
				return true, nil
			}
		}
		return false, nil
	})
	if err != nil {
		return "", fmt.Errorf("waiting for LoadBalancer IP on %s/%s: %w", ns, svcName, err)
	}
	return ip, nil
}

// printSummary prints the post-setup access information.
func printSummary(cfg Config, lbIP string) {
	fmt.Printf("\n\033[1;32mAccess URLs\033[0m (requires /etc/hosts)\n")
	if lbIP == "" {
		lbIP = "<traefik-lb-ip>"
	}
	fmt.Printf("  Traefik Dashboard:  https://traefik.%s\n", cfg.Domain)
	fmt.Printf("  Grafana:            https://grafana.%s  (admin/admin)\n", cfg.Domain)
	fmt.Printf("  Consul UI:          https://consul.%s\n", cfg.Domain)
	fmt.Printf("  MinIO Console:      https://minio.%s  (%s/%s)\n", cfg.Domain, cfg.MinioUser, cfg.MinioPassword)
	fmt.Printf("  Langfuse:           https://langfuse.%s\n", cfg.Domain)
	fmt.Printf("  Memos:              https://memos.%s\n", cfg.Domain)
	fmt.Printf("\n  LoadBalancer IP: %s\n", lbIP)
	fmt.Printf("  Kubeconfig:      %s\n", KindKubeconfigPath(cfg.ClusterName))
}
