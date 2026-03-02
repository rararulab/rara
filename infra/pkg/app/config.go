package app

import (
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi/config"
)

// AppConfig holds all configuration values for the app stack.
type AppConfig struct {
	Namespace string
	Domain    string

	// Backend
	BackendImageRepo   string
	BackendImageTag    string
	BackendPullPolicy  string

	// Frontend
	FrontendImageRepo  string
	FrontendImageTag   string
	FrontendPullPolicy string

	// Consul
	ConsulAddress string
}

// LoadAppConfig reads Pulumi config and returns an AppConfig.
func LoadAppConfig(ctx *pulumi.Context) *AppConfig {
	cfg := config.New(ctx, "rara")

	backendRepo := cfg.Get("backend.imageRepo")
	if backendRepo == "" {
		backendRepo = "ghcr.io/rararulab/rara"
	}
	backendTag := cfg.Get("backend.imageTag")
	if backendTag == "" {
		backendTag = "latest"
	}
	backendPull := cfg.Get("backend.imagePullPolicy")
	if backendPull == "" {
		backendPull = "Always"
	}

	frontendRepo := cfg.Get("frontend.imageRepo")
	if frontendRepo == "" {
		frontendRepo = "ghcr.io/rararulab/rara-web"
	}
	frontendTag := cfg.Get("frontend.imageTag")
	if frontendTag == "" {
		frontendTag = "latest"
	}
	frontendPull := cfg.Get("frontend.imagePullPolicy")
	if frontendPull == "" {
		frontendPull = "Always"
	}

	consulAddr := cfg.Get("consul.address")
	if consulAddr == "" {
		consulAddr = "http://consul-server:8500"
	}

	return &AppConfig{
		Namespace:          cfg.Get("namespace"),
		Domain:             cfg.Get("domain"),
		BackendImageRepo:   backendRepo,
		BackendImageTag:    backendTag,
		BackendPullPolicy:  backendPull,
		FrontendImageRepo:  frontendRepo,
		FrontendImageTag:   frontendTag,
		FrontendPullPolicy: frontendPull,
		ConsulAddress:      consulAddr,
	}
}

// Prefix returns the resource name prefix for the app stack.
func (c *AppConfig) Prefix() string {
	return "rara-app"
}
