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
	"strings"
	"time"

	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/util/wait"
	"k8s.io/client-go/kubernetes"
	k8sscheme "k8s.io/client-go/kubernetes/scheme"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
	"k8s.io/client-go/tools/remotecommand"
)

// SeedConsulKV seeds all required Consul KV pairs by exec-ing into the consul-server-0 pod.
func SeedConsulKV(ctx context.Context, cfg Config, kubeconfigPath string) error {
	rc, err := clientcmd.BuildConfigFromFlags("", kubeconfigPath)
	if err != nil {
		return fmt.Errorf("build rest config: %w", err)
	}
	kc, err := kubernetes.NewForConfig(rc)
	if err != nil {
		return fmt.Errorf("create kubernetes client: %w", err)
	}

	ns := cfg.Namespace
	consulPod := "consul-server-0"

	// Wait for consul to be ready
	if err := Wait("Waiting for Consul to be ready", func() error {
		return waitForConsulReady(ctx, kc, rc, ns, consulPod)
	}); err != nil {
		return err
	}

	kvPairs := buildKVPairs(cfg)

	for key, value := range kvPairs {
		localKey, localValue := key, value
		if err := Wait(fmt.Sprintf("  KV: %s", localKey), func() error {
			return consulPut(ctx, kc, rc, ns, consulPod, localKey, localValue)
		}); err != nil {
			return fmt.Errorf("set consul KV %s: %w", localKey, err)
		}
	}

	OK("Consul KV seed complete")
	return nil
}

// buildKVPairs returns all required Consul KV pairs for the given Config.
func buildKVPairs(cfg Config) map[string]string {
	prefix := cfg.Prefix()

	pairs := map[string]string{
		"rara/config/database/database_url":          fmt.Sprintf("postgres://postgres:%s@%s-postgresql:5432/%s", cfg.PostgresPassword, prefix, cfg.PostgresDatabase),
		"rara/config/database/migration_dir":         "crates/rara-model/migrations",
		"rara/config/http/bind_address":              "0.0.0.0:25555",
		"rara/config/grpc/bind_address":              "0.0.0.0:50051",
		"rara/config/main_service_http_base":         "http://rara-app-backend:25555",
		"rara/config/object_store/endpoint":          fmt.Sprintf("http://%s-minio:9000", prefix),
		"rara/config/object_store/access_key_id":     cfg.MinioUser,
		"rara/config/object_store/secret_access_key": cfg.MinioPassword,
		"rara/config/object_store/bucket":            "rara",
		"rara/config/memory/chroma_url":              fmt.Sprintf("http://%s-chromadb:8000", prefix),
		"rara/config/memory/mem0_base_url":           fmt.Sprintf("http://%s-mem0:8000", prefix),
		"rara/config/memory/memos_base_url":          fmt.Sprintf("http://%s-memos:5230", prefix),
		"rara/config/memory/memos_token":             "",
		"rara/config/memory/hindsight_base_url":      fmt.Sprintf("http://%s-hindsight:8888", prefix),
		"rara/config/memory/hindsight_bank_id":       "default",
		"rara/config/memory/ollama_base_url":         fmt.Sprintf("http://%s-ollama:11434", prefix),
		"rara/config/crawl4ai/base_url":              fmt.Sprintf("http://%s-crawl4ai:11235", prefix),
		"rara/config/telemetry/otlp_endpoint":        fmt.Sprintf("http://%s-alloy:4318/v1/traces", prefix),
		"rara/config/langfuse/host":                  fmt.Sprintf("http://%s-langfuse-web:3000", prefix),
	}

	if cfg.LangfusePublicKey != "" {
		pairs["rara/config/langfuse/public_key"] = cfg.LangfusePublicKey
	}
	if cfg.LangfuseSecretKey != "" {
		pairs["rara/config/langfuse/secret_key"] = cfg.LangfuseSecretKey
	}

	return pairs
}

// waitForConsulReady polls until Consul's leader endpoint returns OK.
func waitForConsulReady(ctx context.Context, kc *kubernetes.Clientset, rc *rest.Config, ns, podName string) error {
	return wait.PollUntilContextTimeout(ctx, 5*time.Second, 5*time.Minute, true, func(ctx context.Context) (bool, error) {
		stdout, _, err := execInPod(ctx, kc, rc, ns, podName, "consul",
			[]string{"sh", "-c", "curl -sf http://127.0.0.1:8500/v1/status/leader"})
		if err != nil {
			return false, nil
		}
		return strings.TrimSpace(stdout) != "", nil
	})
}

// consulPut sets a Consul KV pair by exec-ing into the consul pod.
func consulPut(ctx context.Context, kc *kubernetes.Clientset, rc *rest.Config, ns, podName, key, value string) error {
	// Escape single quotes in the value for shell safety
	escaped := strings.ReplaceAll(value, "'", `'"'"'`)
	cmd := fmt.Sprintf("curl -sf -X PUT -d '%s' http://127.0.0.1:8500/v1/kv/%s", escaped, key)
	_, stderr, err := execInPod(ctx, kc, rc, ns, podName, "consul", []string{"sh", "-c", cmd})
	if err != nil {
		return fmt.Errorf("exec failed: %w (stderr: %s)", err, stderr)
	}
	return nil
}

// execInPod executes a command in a pod container and returns stdout, stderr, error.
func execInPod(ctx context.Context, kc *kubernetes.Clientset, rc *rest.Config, ns, podName, containerName string, command []string) (string, string, error) {
	req := kc.CoreV1().RESTClient().Post().
		Resource("pods").
		Name(podName).
		Namespace(ns).
		SubResource("exec").
		VersionedParams(&corev1.PodExecOptions{
			Container: containerName,
			Command:   command,
			Stdin:     false,
			Stdout:    true,
			Stderr:    true,
			TTY:       false,
		}, k8sscheme.ParameterCodec)

	executor, err := remotecommand.NewSPDYExecutor(rc, "POST", req.URL())
	if err != nil {
		return "", "", fmt.Errorf("create SPDY executor: %w", err)
	}

	var stdout, stderr bytes.Buffer
	err = executor.StreamWithContext(ctx, remotecommand.StreamOptions{
		Stdout: &stdout,
		Stderr: &stderr,
	})
	return stdout.String(), stderr.String(), err
}
