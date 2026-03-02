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
