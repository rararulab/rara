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
