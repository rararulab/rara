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
	corev1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/core/v1"
	metav1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/meta/v1"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// Run is the entry point for the infra stack.
func Run(ctx *pulumi.Context) error {
	cfg := LoadInfraConfig(ctx)

	// Ensure namespace exists
	ns, err := corev1.NewNamespace(ctx, cfg.Namespace, &corev1.NamespaceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name: pulumi.String(cfg.Namespace),
		},
	})
	if err != nil {
		return err
	}

	// Layer 1: Network + Certificates
	network, err := DeployNetwork(ctx, cfg)
	if err != nil {
		return err
	}

	// Layer 2: Data + Config
	data, err := DeployData(ctx, cfg)
	if err != nil {
		return err
	}

	// Layer 3: Custom services (pure K8s)
	services, err := DeployServices(ctx, cfg)
	if err != nil {
		return err
	}

	// Layer 3: Observability (Helm)
	obs, err := DeployObservability(ctx, cfg)
	if err != nil {
		return err
	}

	// Keel — automatic image update controller
	_, err = DeployKeel(ctx, cfg)
	if err != nil {
		return err
	}

	// Consul KV seeding (depends on everything)
	consulDeps := []pulumi.Resource{
		ns,
		network.Traefik,
		network.CertManager,
		data.PostgreSQL,
		data.MinIO,
		data.Consul,
		services.ChromaDB,
		services.Crawl4AI,
		services.Memos,
		services.Hindsight,
		services.Mem0,
		services.Ollama,
		obs.PrometheusStack,
		obs.Tempo,
		obs.Alloy,
		obs.Quickwit,
		obs.Langfuse,
	}
	if err := SeedConsulKV(ctx, cfg, consulDeps); err != nil {
		return err
	}

	// Exports
	ctx.Export("namespace", pulumi.String(cfg.Namespace))
	ctx.Export("domain", pulumi.String(cfg.Domain))
	ctx.Export("consulAddress", pulumi.String("http://consul-server:8500"))

	return nil
}
