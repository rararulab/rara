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

package infra

import (
	"fmt"

	"github.com/pulumi/pulumi-command/sdk/go/command/local"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// SeedConsulKV seeds Consul KV with configuration values via kubectl exec.
func SeedConsulKV(ctx *pulumi.Context, cfg *InfraConfig, deps []pulumi.Resource) error {
	prefix := cfg.Prefix()
	ns := cfg.Namespace

	kvPairs := map[string]string{
		"rara/config/database/database_url":        fmt.Sprintf("postgres://postgres:%s@%s-postgresql:5432/%s", cfg.PostgresPassword, prefix, cfg.PostgresDatabase),
		"rara/config/database/migration_dir":       "crates/rara-model/migrations",
		"rara/config/http/bind_address":            "0.0.0.0:25555",
		"rara/config/grpc/bind_address":            "0.0.0.0:50051",
		"rara/config/main_service_http_base":       "http://rara-app-backend:25555",
		"rara/config/object_store/endpoint":        fmt.Sprintf("http://%s-minio:9000", prefix),
		"rara/config/object_store/access_key_id":   cfg.MinioRootUser,
		"rara/config/object_store/secret_access_key": cfg.MinioRootPassword,
		"rara/config/object_store/bucket":          "rara",
		"rara/config/memory/chroma_url":            fmt.Sprintf("http://%s-chromadb:8000", prefix),
		"rara/config/memory/mem0_base_url":         fmt.Sprintf("http://%s-mem0:8000", prefix),
		"rara/config/memory/memos_base_url":        fmt.Sprintf("http://%s-memos:5230", prefix),
		"rara/config/memory/memos_token":           "",
		"rara/config/memory/hindsight_base_url":    fmt.Sprintf("http://%s-hindsight:8888", prefix),
		"rara/config/memory/hindsight_bank_id":     "default",
		"rara/config/memory/ollama_base_url":       fmt.Sprintf("http://%s-ollama:11434", prefix),
		"rara/config/crawl4ai/base_url":            fmt.Sprintf("http://%s-crawl4ai:11235", prefix),
		"rara/config/telemetry/otlp_endpoint":      fmt.Sprintf("http://%s-alloy:4318/v1/traces", prefix),
		"rara/config/langfuse/host":                fmt.Sprintf("http://%s-langfuse-web:3000", prefix),
	}

	// Add Langfuse keys only if configured
	if cfg.LangfusePublicKey != "" {
		kvPairs["rara/config/langfuse/public_key"] = cfg.LangfusePublicKey
	}
	if cfg.LangfuseSecretKey != "" {
		kvPairs["rara/config/langfuse/secret_key"] = cfg.LangfuseSecretKey
	}

	// Build a single shell script that seeds all KV pairs
	script := "#!/bin/sh\nset -e\n\n"
	script += fmt.Sprintf("CONSUL_POD=\"consul-server-0\"\nNS=\"%s\"\n\n", ns)
	script += "# Wait for Consul to be ready\n"
	script += "echo 'Waiting for Consul...'\n"
	script += "for i in $(seq 1 60); do\n"
	script += "  if kubectl exec $CONSUL_POD -n $NS -- curl -sf http://127.0.0.1:8500/v1/status/leader > /dev/null 2>&1; then\n"
	script += "    echo 'Consul is ready.'\n"
	script += "    break\n"
	script += "  fi\n"
	script += "  if [ $i -eq 60 ]; then\n"
	script += "    echo 'ERROR: Consul not ready after 120s'\n"
	script += "    exit 1\n"
	script += "  fi\n"
	script += "  sleep 2\n"
	script += "done\n\n"
	script += "echo 'Seeding Consul KV...'\n"

	for key, value := range kvPairs {
		script += fmt.Sprintf("echo 'Setting %s'\n", key)
		script += fmt.Sprintf("kubectl exec $CONSUL_POD -n $NS -- curl -sf -X PUT -d '%s' http://127.0.0.1:8500/v1/kv/%s > /dev/null\n", value, key)
	}
	script += "\necho 'Consul KV seed complete.'\n"

	_, err := local.NewCommand(ctx, fmt.Sprintf("%s-consul-kv-seed", prefix), &local.CommandArgs{
		Create: pulumi.String(script),
	}, pulumi.DependsOn(deps))

	return err
}
