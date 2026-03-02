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

	helmv4 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/helm/v4"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// ObservabilityResult holds references to observability layer resources.
type ObservabilityResult struct {
	PrometheusStack *helmv4.Chart
	Tempo           *helmv4.Chart
	Alloy           *helmv4.Chart
	Quickwit        *helmv4.Chart
	Langfuse        *helmv4.Chart
}

// DeployObservability deploys kube-prometheus-stack, Tempo, Alloy, Quickwit, and Langfuse.
func DeployObservability(ctx *pulumi.Context, cfg *InfraConfig) (*ObservabilityResult, error) {
	prefix := cfg.Prefix()
	ns := cfg.Namespace

	// --- kube-prometheus-stack ---
	promStack, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-kube-prometheus-stack", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("kube-prometheus-stack"),
		Version: pulumi.String("82.2.1"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://prometheus-community.github.io/helm-charts"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"grafana": pulumi.Map{
				"enabled":       pulumi.Bool(true),
				"adminUser":     pulumi.String("admin"),
				"adminPassword": pulumi.String("admin"),
				"grafana.ini": pulumi.Map{
					"auth.anonymous": pulumi.Map{
						"enabled":  pulumi.Bool(true),
						"org_role": pulumi.String("Admin"),
					},
					"auth": pulumi.Map{
						"disable_login_form": pulumi.Bool(true),
					},
				},
				"initChownData": pulumi.Map{
					"enabled": pulumi.Bool(false),
				},
				"persistence": pulumi.Map{
					"enabled": pulumi.Bool(true),
					"size":    pulumi.String("512Mi"),
				},
				"sidecar": pulumi.Map{
					"datasources": pulumi.Map{
						"enabled": pulumi.Bool(true),
					},
					"dashboards": pulumi.Map{
						"enabled":         pulumi.Bool(true),
						"label":           pulumi.String("grafana_dashboard"),
						"labelValue":      pulumi.String("1"),
						"searchNamespace": pulumi.String("ALL"),
					},
				},
				"additionalDataSources": pulumi.MapArray{
					pulumi.Map{
						"name":   pulumi.String("Tempo"),
						"type":   pulumi.String("tempo"),
						"access": pulumi.String("proxy"),
						"url":    pulumi.String(fmt.Sprintf("http://%s-tempo:3100", prefix)),
						"jsonData": pulumi.Map{
							"tracesToLogsV2": pulumi.Map{
								"datasourceUid": pulumi.String("quickwit"),
							},
							"nodeGraph": pulumi.Map{
								"enabled": pulumi.Bool(true),
							},
						},
					},
					pulumi.Map{
						"name":   pulumi.String("Quickwit"),
						"type":   pulumi.String("quickwit-quickwit-datasource"),
						"access": pulumi.String("proxy"),
						"url":    pulumi.String(fmt.Sprintf("http://%s-quickwit-searcher:7280/api/v1", prefix)),
					},
				},
				"resources": pulumi.Map{
					"requests": pulumi.Map{
						"cpu":    pulumi.String("100m"),
						"memory": pulumi.String("128Mi"),
					},
					"limits": pulumi.Map{
						"cpu":    pulumi.String("500m"),
						"memory": pulumi.String("512Mi"),
					},
				},
			},
			"prometheus": pulumi.Map{
				"prometheusSpec": pulumi.Map{
					"enableRemoteWriteReceiver": pulumi.Bool(true),
					"retention":                 pulumi.String("2d"),
					"resources": pulumi.Map{
						"requests": pulumi.Map{
							"cpu":    pulumi.String("100m"),
							"memory": pulumi.String("256Mi"),
						},
						"limits": pulumi.Map{
							"cpu":    pulumi.String("1"),
							"memory": pulumi.String("1Gi"),
						},
					},
					"storageSpec": pulumi.Map{
						"volumeClaimTemplate": pulumi.Map{
							"spec": pulumi.Map{
								"accessModes": pulumi.StringArray{pulumi.String("ReadWriteOnce")},
								"resources": pulumi.Map{
									"requests": pulumi.Map{
										"storage": pulumi.String("2Gi"),
									},
								},
							},
						},
					},
				},
			},
			"alertmanager": pulumi.Map{
				"enabled": pulumi.Bool(true),
				"alertmanagerSpec": pulumi.Map{
					"resources": pulumi.Map{
						"requests": pulumi.Map{
							"cpu":    pulumi.String("50m"),
							"memory": pulumi.String("64Mi"),
						},
						"limits": pulumi.Map{
							"cpu":    pulumi.String("250m"),
							"memory": pulumi.String("256Mi"),
						},
					},
					"storage": pulumi.Map{
						"volumeClaimTemplate": pulumi.Map{
							"spec": pulumi.Map{
								"accessModes": pulumi.StringArray{pulumi.String("ReadWriteOnce")},
								"resources": pulumi.Map{
									"requests": pulumi.Map{
										"storage": pulumi.String("512Mi"),
									},
								},
							},
						},
					},
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// --- Tempo ---
	tempo, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-tempo", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("tempo"),
		Version: pulumi.String("1.24.4"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://grafana.github.io/helm-charts"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"tempo": pulumi.Map{
				"receivers": pulumi.Map{
					"otlp": pulumi.Map{
						"protocols": pulumi.Map{
							"grpc": pulumi.Map{
								"endpoint": pulumi.String("0.0.0.0:4317"),
							},
							"http": pulumi.Map{
								"endpoint": pulumi.String("0.0.0.0:4318"),
							},
						},
					},
				},
				"storage": pulumi.Map{
					"trace": pulumi.Map{
						"backend": pulumi.String("local"),
						"local": pulumi.Map{
							"path": pulumi.String("/var/tempo/traces"),
						},
						"wal": pulumi.Map{
							"path": pulumi.String("/var/tempo/wal"),
						},
					},
				},
				"resources": pulumi.Map{
					"requests": pulumi.Map{
						"cpu":    pulumi.String("100m"),
						"memory": pulumi.String("128Mi"),
					},
					"limits": pulumi.Map{
						"cpu":    pulumi.String("500m"),
						"memory": pulumi.String("512Mi"),
					},
				},
			},
			"persistence": pulumi.Map{
				"enabled": pulumi.Bool(true),
				"size":    pulumi.String("1Gi"),
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// --- Alloy (OpenTelemetry collector) ---
	alloyConfig := fmt.Sprintf(`// OTLP receiver for traces, metrics, and logs
otelcol.receiver.otlp "default" {
  grpc {
    endpoint = "0.0.0.0:4317"
  }
  http {
    endpoint = "0.0.0.0:4318"
  }
  output {
    traces  = [otelcol.exporter.otlp.tempo.input]
    metrics = [otelcol.exporter.prometheus.default.input]
    logs    = [otelcol.exporter.otlphttp.quickwit.input]
  }
}

// Export traces to Tempo
otelcol.exporter.otlp "tempo" {
  client {
    endpoint = "%s-tempo:4317"
    tls {
      insecure = true
    }
  }
}

// Export metrics to Prometheus remote write
otelcol.exporter.prometheus "default" {
  forward_to = [prometheus.remote_write.default.receiver]
}

prometheus.remote_write "default" {
  endpoint {
    url = "http://%s-kube-prometheus-prometheus:9090/api/v1/write"
  }
}

// Export logs to Quickwit via OTLP HTTP
otelcol.exporter.otlphttp "quickwit" {
  client {
    endpoint = "http://%s-quickwit-indexer:7281"
  }
}
`, prefix, prefix, prefix)

	alloy, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-alloy", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("alloy"),
		Version: pulumi.String("1.6.0"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://grafana.github.io/helm-charts"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"alloy": pulumi.Map{
				"configMap": pulumi.Map{
					"content": pulumi.String(alloyConfig),
				},
				"resources": pulumi.Map{
					"requests": pulumi.Map{
						"cpu":    pulumi.String("50m"),
						"memory": pulumi.String("64Mi"),
					},
					"limits": pulumi.Map{
						"cpu":    pulumi.String("250m"),
						"memory": pulumi.String("256Mi"),
					},
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// --- Quickwit ---
	quickwit, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-quickwit", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("quickwit"),
		Version: pulumi.String("0.7.21"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://helm.quickwit.io"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"config": pulumi.Map{
				"default_index_root_uri": pulumi.String("s3://quickwit/indexes"),
				"storage": pulumi.Map{
					"s3": pulumi.Map{
						"endpoint":               pulumi.String(fmt.Sprintf("http://%s-minio:9000", prefix)),
						"access_key_id":           pulumi.String(cfg.MinioRootUser),
						"secret_access_key":       pulumi.String(cfg.MinioRootPassword),
						"force_path_style_access": pulumi.Bool(true),
						"region":                  pulumi.String("us-east-1"),
					},
				},
			},
			"searcher": pulumi.Map{
				"replicaCount": pulumi.Int(1),
				"resources": pulumi.Map{
					"requests": pulumi.Map{"cpu": pulumi.String("100m"), "memory": pulumi.String("256Mi")},
					"limits":   pulumi.Map{"cpu": pulumi.String("500m"), "memory": pulumi.String("512Mi")},
				},
			},
			"indexer": pulumi.Map{
				"replicaCount": pulumi.Int(1),
				"resources": pulumi.Map{
					"requests": pulumi.Map{"cpu": pulumi.String("100m"), "memory": pulumi.String("256Mi")},
					"limits":   pulumi.Map{"cpu": pulumi.String("500m"), "memory": pulumi.String("512Mi")},
				},
			},
			"metastore": pulumi.Map{
				"replicaCount": pulumi.Int(1),
				"resources": pulumi.Map{
					"requests": pulumi.Map{"cpu": pulumi.String("100m"), "memory": pulumi.String("128Mi")},
					"limits":   pulumi.Map{"cpu": pulumi.String("250m"), "memory": pulumi.String("256Mi")},
				},
			},
			"control_plane": pulumi.Map{
				"replicaCount": pulumi.Int(1),
				"resources": pulumi.Map{
					"requests": pulumi.Map{"cpu": pulumi.String("100m"), "memory": pulumi.String("128Mi")},
					"limits":   pulumi.Map{"cpu": pulumi.String("250m"), "memory": pulumi.String("256Mi")},
				},
			},
			"janitor": pulumi.Map{
				"enabled": pulumi.Bool(true),
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// --- Langfuse ---
	langfuse, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-langfuse", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("langfuse"),
		Version: pulumi.String("1.5.20"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://langfuse.github.io/langfuse-k8s"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"langfuse": pulumi.Map{
				"nextauth": pulumi.Map{
					"url": pulumi.String(fmt.Sprintf("https://langfuse.%s", cfg.Domain)),
					"secret": pulumi.Map{
						"value": pulumi.String("rara-langfuse-nextauth-secret-change-me"),
					},
				},
				"salt": pulumi.Map{
					"value": pulumi.String("rara-langfuse-salt-change-me"),
				},
				"features": pulumi.Map{
					"telemetryEnabled": pulumi.Bool(false),
				},
				"resources": pulumi.Map{
					"requests": pulumi.Map{"cpu": pulumi.String("200m"), "memory": pulumi.String("512Mi")},
					"limits":   pulumi.Map{"cpu": pulumi.String("1"), "memory": pulumi.String("1Gi")},
				},
			},
			"postgresql": pulumi.Map{
				"deploy":           pulumi.Bool(true),
				"fullnameOverride": pulumi.String(fmt.Sprintf("%s-langfuse-pg", prefix)),
				"nameOverride":     pulumi.String("langfuse-pg"),
				"host":             pulumi.String(fmt.Sprintf("%s-langfuse-pg", prefix)),
				"auth": pulumi.Map{
					"password": pulumi.String("langfuse"),
				},
			},
			"clickhouse": pulumi.Map{
				"deploy": pulumi.Bool(true),
				"host":   pulumi.String(fmt.Sprintf("%s-clickhouse", prefix)),
				"auth": pulumi.Map{
					"password": pulumi.String("clickhouse"),
				},
				"persistence": pulumi.Map{
					"enabled": pulumi.Bool(true),
					"size":    pulumi.String("2Gi"),
				},
				"resources": pulumi.Map{
					"requests": pulumi.Map{"cpu": pulumi.String("100m"), "memory": pulumi.String("256Mi")},
				},
			},
			"redis": pulumi.Map{
				"deploy": pulumi.Bool(true),
				"host":   pulumi.String(fmt.Sprintf("%s-redis-primary", prefix)),
				"auth": pulumi.Map{
					"enabled": pulumi.Bool(false),
				},
				"master": pulumi.Map{
					"persistence": pulumi.Map{
						"enabled": pulumi.Bool(true),
						"size":    pulumi.String("256Mi"),
					},
					"resources": pulumi.Map{
						"requests": pulumi.Map{"cpu": pulumi.String("50m"), "memory": pulumi.String("64Mi")},
					},
				},
			},
			"s3": pulumi.Map{
				"deploy":         pulumi.Bool(false),
				"bucket":         pulumi.String("langfuse"),
				"region":         pulumi.String("us-east-1"),
				"endpoint":       pulumi.String(fmt.Sprintf("http://%s-minio:9000", prefix)),
				"forcePathStyle": pulumi.Bool(true),
				"accessKeyId": pulumi.Map{
					"value": pulumi.String(cfg.MinioRootUser),
				},
				"secretAccessKey": pulumi.Map{
					"value": pulumi.String(cfg.MinioRootPassword),
				},
				"eventUpload": pulumi.Map{
					"bucket": pulumi.String("langfuse"),
				},
				"batchExport": pulumi.Map{
					"bucket": pulumi.String("langfuse"),
				},
				"mediaUpload": pulumi.Map{
					"bucket": pulumi.String("langfuse"),
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return &ObservabilityResult{
		PrometheusStack: promStack,
		Tempo:           tempo,
		Alloy:           alloy,
		Quickwit:        quickwit,
		Langfuse:        langfuse,
	}, nil
}
