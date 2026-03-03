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

// Up brings up the complete local rara environment.
func Up(ctx context.Context, cfg Config) error {
	fmt.Printf("\033[1;32m==> rara local setup\033[0m\n")
	fmt.Printf("    cluster: %s  namespace: %s  domain: %s\n\n", cfg.ClusterName, cfg.Namespace, cfg.Domain)

	// Step 1: kind cluster
	Step(1, totalSteps, "Ensure kind cluster")
	kubeconfigPath, err := EnsureCluster(ctx, cfg)
	if err != nil {
		return fmt.Errorf("ensure cluster: %w", err)
	}
	OK(fmt.Sprintf("kubeconfig: %s", kubeconfigPath))

	// Step 2: MetalLB
	Step(2, totalSteps, "Install MetalLB (LoadBalancer support)")
	rc, err := clientcmd.BuildConfigFromFlags("", kubeconfigPath)
	if err != nil {
		return fmt.Errorf("build rest config: %w", err)
	}
	if err := InstallMetalLB(ctx, rc); err != nil {
		return fmt.Errorf("install metallb: %w", err)
	}

	// Step 3: Helm charts (infra stack)
	Step(3, totalSteps, "Install infrastructure Helm charts")
	if err := InstallHelmCharts(ctx, cfg, kubeconfigPath); err != nil {
		return fmt.Errorf("install helm charts: %w", err)
	}

	// Step 4: Custom K8s services
	Step(4, totalSteps, "Deploy custom K8s services")
	if err := DeployCustomServices(ctx, cfg, kubeconfigPath); err != nil {
		return fmt.Errorf("deploy custom services: %w", err)
	}

	// Step 5: Seed Consul KV
	Step(5, totalSteps, "Seed Consul KV")
	if err := SeedConsulKV(ctx, cfg, kubeconfigPath); err != nil {
		return fmt.Errorf("seed consul kv: %w", err)
	}

	// Step 6: Get LoadBalancer IP (Traefik)
	Step(6, totalSteps, "Discover Traefik LoadBalancer IP")
	lbIP, err := GetTraefikIP(ctx, rc, cfg)
	if err != nil {
		Warn(fmt.Sprintf("Could not determine Traefik IP: %v", err))
		Warn("You may need to update /etc/hosts manually")
		lbIP = ""
	} else {
		OK(fmt.Sprintf("Traefik LoadBalancer IP: %s", lbIP))
	}

	// Step 7: /etc/hosts
	Step(7, totalSteps, "Update /etc/hosts")
	if lbIP != "" {
		if err := Wait("Writing /etc/hosts entries", func() error {
			return AddHostsEntries(lbIP, cfg.Domain)
		}); err != nil {
			Warn(fmt.Sprintf("Could not update /etc/hosts (try running with sudo): %v", err))
			fmt.Println("\nAdd the following entries to /etc/hosts manually:")
			PrintHostsBlock(lbIP, cfg.Domain)
		}
	} else {
		Info("Skipping /etc/hosts (no LoadBalancer IP)")
	}

	// Step 8: Summary
	Step(8, totalSteps, "Setup complete")
	printSummary(cfg, lbIP)

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
		return DeleteCluster(cfg.ClusterName)
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
