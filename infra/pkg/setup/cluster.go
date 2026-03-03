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
	"context"
	"fmt"
	"os"
	"path/filepath"

	"sigs.k8s.io/kind/pkg/cluster"
)

// ClusterExists returns true if a kind cluster with the given name exists.
func ClusterExists(name string) (bool, error) {
	provider := cluster.NewProvider()
	clusters, err := provider.List()
	if err != nil {
		return false, fmt.Errorf("list kind clusters: %w", err)
	}
	for _, c := range clusters {
		if c == name {
			return true, nil
		}
	}
	return false, nil
}

// EnsureCluster creates a kind cluster if it doesn't already exist.
// Returns the kubeconfig path.
func EnsureCluster(_ context.Context, cfg Config, send Sender) (string, error) {
	kubeconfigPath := KindKubeconfigPath(cfg.ClusterName)

	exists, err := ClusterExists(cfg.ClusterName)
	if err != nil {
		return "", err
	}

	if exists {
		send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("kind cluster %q already exists", cfg.ClusterName)})
		provider := cluster.NewProvider()
		if err := provider.ExportKubeConfig(cfg.ClusterName, kubeconfigPath, false); err != nil {
			return "", fmt.Errorf("export kubeconfig: %w", err)
		}
		return kubeconfigPath, nil
	}

	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Creating kind cluster %q...", cfg.ClusterName)})
	provider := cluster.NewProvider()
	if err := provider.Create(
		cfg.ClusterName,
		cluster.CreateWithKubeconfigPath(kubeconfigPath),
	); err != nil {
		return "", fmt.Errorf("create kind cluster: %w", err)
	}

	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("kind cluster %q created", cfg.ClusterName)})
	return kubeconfigPath, nil
}

// DeleteCluster deletes the kind cluster.
func DeleteCluster(name string, send Sender) error {
	exists, err := ClusterExists(name)
	if err != nil {
		return err
	}
	if !exists {
		send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("cluster %q not found, nothing to delete", name)})
		return nil
	}

	provider := cluster.NewProvider()
	if err := provider.Delete(name, KindKubeconfigPath(name)); err != nil {
		return fmt.Errorf("delete kind cluster: %w", err)
	}
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("kind cluster %q deleted", name)})
	return nil
}

// GetNodeContainerIP returns the internal IPv4 of the kind control-plane node container.
func GetNodeContainerIP(clusterName string) (string, error) {
	provider := cluster.NewProvider()
	nodes, err := provider.ListNodes(clusterName)
	if err != nil {
		return "", fmt.Errorf("list nodes: %w", err)
	}
	for _, node := range nodes {
		ipv4, _, err := node.IP()
		if err == nil && ipv4 != "" {
			return ipv4, nil
		}
	}
	return "", fmt.Errorf("no nodes found in cluster %q", clusterName)
}

// KindKubeconfigPath returns the kubeconfig path for a kind cluster.
func KindKubeconfigPath(clusterName string) string {
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".kube", fmt.Sprintf("kind-%s.yaml", clusterName))
}
