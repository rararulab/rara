package infra

import (
	"fmt"

	helmv4 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/helm/v4"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// DataResult holds references to data layer resources.
type DataResult struct {
	PostgreSQL *helmv4.Chart
	MinIO      *helmv4.Chart
	Consul     *helmv4.Chart
}

// DeployData deploys PostgreSQL, MinIO, and Consul Helm charts.
func DeployData(ctx *pulumi.Context, cfg *InfraConfig) (*DataResult, error) {
	prefix := cfg.Prefix()
	ns := cfg.Namespace

	// --- PostgreSQL (Bitnami with pgmq image) ---
	pg, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-postgresql", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("postgresql"),
		Version: pulumi.String("18.4.0"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://charts.bitnami.com/bitnami"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"global": pulumi.Map{
				"security": pulumi.Map{
					"allowInsecureImages": pulumi.Bool(true),
				},
			},
			"image": pulumi.Map{
				"registry":   pulumi.String("ghcr.io"),
				"repository": pulumi.String("pgmq/pg18-pgmq"),
				"tag":        pulumi.String("v1.10.0"),
			},
			"auth": pulumi.Map{
				"postgresPassword": pulumi.String(cfg.PostgresPassword),
				"database":         pulumi.String(cfg.PostgresDatabase),
				"usePasswordFiles": pulumi.Bool(false),
			},
			"volumePermissions": pulumi.Map{
				"enabled": pulumi.Bool(true),
			},
			"primary": pulumi.Map{
				"podSecurityContext": pulumi.Map{
					"enabled": pulumi.Bool(true),
					"fsGroup": pulumi.Int(999),
				},
				"containerSecurityContext": pulumi.Map{
					"enabled":              pulumi.Bool(true),
					"runAsUser":            pulumi.Int(999),
					"runAsGroup":           pulumi.Int(999),
					"readOnlyRootFilesystem": pulumi.Bool(false),
				},
				"persistence": pulumi.Map{
					"enabled": pulumi.Bool(true),
					"size":    pulumi.String("2Gi"),
				},
				"resources": pulumi.Map{
					"requests": pulumi.Map{
						"cpu":    pulumi.String("100m"),
						"memory": pulumi.String("256Mi"),
					},
					"limits": pulumi.Map{
						"cpu":    pulumi.String("500m"),
						"memory": pulumi.String("512Mi"),
					},
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// --- MinIO ---
	minio, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-minio", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("minio"),
		Version: pulumi.String("5.4.0"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://charts.min.io"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"mode":         pulumi.String("standalone"),
			"rootUser":     pulumi.String(cfg.MinioRootUser),
			"rootPassword": pulumi.String(cfg.MinioRootPassword),
			"consoleService": pulumi.Map{
				"type": pulumi.String("ClusterIP"),
				"port": pulumi.Int(9001),
			},
			"service": pulumi.Map{
				"type": pulumi.String("ClusterIP"),
				"port": pulumi.Int(9000),
			},
			"buckets": pulumi.MapArray{
				pulumi.Map{"name": pulumi.String("rara"), "policy": pulumi.String("none"), "purge": pulumi.Bool(false)},
				pulumi.Map{"name": pulumi.String("langfuse"), "policy": pulumi.String("none"), "purge": pulumi.Bool(false)},
				pulumi.Map{"name": pulumi.String("quickwit"), "policy": pulumi.String("none"), "purge": pulumi.Bool(false)},
			},
			"persistence": pulumi.Map{
				"enabled": pulumi.Bool(true),
				"size":    pulumi.String("2Gi"),
			},
			"resources": pulumi.Map{
				"requests": pulumi.Map{
					"cpu":    pulumi.String("100m"),
					"memory": pulumi.String("256Mi"),
				},
				"limits": pulumi.Map{
					"cpu":    pulumi.String("500m"),
					"memory": pulumi.String("512Mi"),
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// --- Consul ---
	consul, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-consul", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("consul"),
		Version: pulumi.String("1.9.3"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://helm.releases.hashicorp.com"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"global": pulumi.Map{
				"name":       pulumi.String("consul"),
				"datacenter": pulumi.String("rara-dc1"),
			},
			"server": pulumi.Map{
				"replicas": pulumi.Int(1),
				"storage":  pulumi.String("1Gi"),
				"resources": pulumi.Map{
					"requests": pulumi.Map{
						"cpu":    pulumi.String("100m"),
						"memory": pulumi.String("128Mi"),
					},
					"limits": pulumi.Map{
						"cpu":    pulumi.String("500m"),
						"memory": pulumi.String("256Mi"),
					},
				},
			},
			"client": pulumi.Map{
				"enabled": pulumi.Bool(true),
			},
			"ui": pulumi.Map{
				"enabled": pulumi.Bool(true),
			},
			"connectInject": pulumi.Map{
				"enabled": pulumi.Bool(false),
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return &DataResult{
		PostgreSQL: pg,
		MinIO:      minio,
		Consul:     consul,
	}, nil
}
