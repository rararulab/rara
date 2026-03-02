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
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi/config"
)

// InfraConfig holds all configuration values for the infra stack.
type InfraConfig struct {
	Namespace string
	Domain    string

	// PostgreSQL
	PostgresPassword string
	PostgresDatabase string

	// MinIO
	MinioRootUser     string
	MinioRootPassword string

	// Langfuse
	LangfusePublicKey string
	LangfuseSecretKey string

	// Hindsight
	HindsightLLMProvider string
	HindsightLLMModel    string
	HindsightLLMBaseURL  string

	// Mem0
	Mem0OllamaBaseURL string
	Mem0OllamaModel   string
}

// LoadInfraConfig reads Pulumi config and returns an InfraConfig.
func LoadInfraConfig(ctx *pulumi.Context) *InfraConfig {
	cfg := config.New(ctx, "rara")

	return &InfraConfig{
		Namespace:            cfg.Get("namespace"),
		Domain:               cfg.Get("domain"),
		PostgresPassword:     cfg.Get("postgresql.password"),
		PostgresDatabase:     cfg.Get("postgresql.database"),
		MinioRootUser:        cfg.Get("minio.rootUser"),
		MinioRootPassword:    cfg.Get("minio.rootPassword"),
		LangfusePublicKey:    cfg.Get("langfuse.publicKey"),
		LangfuseSecretKey:    cfg.Get("langfuse.secretKey"),
		HindsightLLMProvider: cfg.Get("hindsight.llmProvider"),
		HindsightLLMModel:    cfg.Get("hindsight.llmModel"),
		HindsightLLMBaseURL:  cfg.Get("hindsight.llmBaseUrl"),
		Mem0OllamaBaseURL:    cfg.Get("mem0.ollamaBaseUrl"),
		Mem0OllamaModel:      cfg.Get("mem0.ollamaModel"),
	}
}

// Prefix returns the resource name prefix for the infra stack.
func (c *InfraConfig) Prefix() string {
	return "rara-infra"
}
