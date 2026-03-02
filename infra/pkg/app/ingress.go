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
	"fmt"

	metav1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/meta/v1"
	"github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/apiextensions"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// DeployIngress creates Traefik IngressRoute CRDs and a Prometheus ServiceMonitor.
func DeployIngress(ctx *pulumi.Context, cfg *AppConfig) error {
	prefix := cfg.Prefix()
	ns := cfg.Namespace
	domain := cfg.Domain
	tlsSecret := "rara-infra-wildcard-tls"

	// --- IngressRoute: Frontend (app.rara.local) ---
	_, err := apiextensions.NewCustomResource(ctx, fmt.Sprintf("%s-frontend-ingress", prefix), &apiextensions.CustomResourceArgs{
		ApiVersion: pulumi.String("traefik.io/v1alpha1"),
		Kind:       pulumi.String("IngressRoute"),
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-frontend", prefix)),
			Namespace: pulumi.String(ns),
		},
		OtherFields: map[string]interface{}{
			"spec": map[string]interface{}{
				"entryPoints": []string{"websecure"},
				"routes": []map[string]interface{}{
					{
						"match": fmt.Sprintf("Host(`app.%s`)", domain),
						"kind":  "Rule",
						"services": []map[string]interface{}{
							{
								"name": fmt.Sprintf("%s-frontend", prefix),
								"port": 80,
							},
						},
					},
				},
				"tls": map[string]interface{}{
					"secretName": tlsSecret,
				},
			},
		},
	})
	if err != nil {
		return err
	}

	// --- IngressRoute: Backend API (api.rara.local) ---
	_, err = apiextensions.NewCustomResource(ctx, fmt.Sprintf("%s-backend-ingress", prefix), &apiextensions.CustomResourceArgs{
		ApiVersion: pulumi.String("traefik.io/v1alpha1"),
		Kind:       pulumi.String("IngressRoute"),
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-backend", prefix)),
			Namespace: pulumi.String(ns),
		},
		OtherFields: map[string]interface{}{
			"spec": map[string]interface{}{
				"entryPoints": []string{"websecure"},
				"routes": []map[string]interface{}{
					{
						"match": fmt.Sprintf("Host(`api.%s`)", domain),
						"kind":  "Rule",
						"services": []map[string]interface{}{
							{
								"name": fmt.Sprintf("%s-backend", prefix),
								"port": 25555,
							},
						},
					},
				},
				"tls": map[string]interface{}{
					"secretName": tlsSecret,
				},
			},
		},
	})
	if err != nil {
		return err
	}

	// --- ServiceMonitor (Prometheus CRD) ---
	backendLabels := map[string]string{
		"app.kubernetes.io/name":      "rara-app",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "backend",
	}

	_, err = apiextensions.NewCustomResource(ctx, fmt.Sprintf("%s-backend-monitor", prefix), &apiextensions.CustomResourceArgs{
		ApiVersion: pulumi.String("monitoring.coreos.com/v1"),
		Kind:       pulumi.String("ServiceMonitor"),
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-backend", prefix)),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(backendLabels),
		},
		OtherFields: map[string]interface{}{
			"spec": map[string]interface{}{
				"selector": map[string]interface{}{
					"matchLabels": backendLabels,
				},
				"endpoints": []map[string]interface{}{
					{
						"port":          "http",
						"path":          "/metrics",
						"interval":      "30s",
						"scrapeTimeout": "10s",
					},
				},
			},
		},
	})
	if err != nil {
		return err
	}

	return nil
}
