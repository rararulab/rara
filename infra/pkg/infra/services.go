package infra

import (
	"fmt"

	appsv1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/apps/v1"
	corev1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/core/v1"
	metav1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/meta/v1"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// ServicesResult holds references to custom K8s service resources.
type ServicesResult struct {
	ChromaDB  *appsv1.Deployment
	Crawl4AI  *appsv1.Deployment
	Memos     *appsv1.Deployment
	Hindsight *appsv1.Deployment
	Mem0      *appsv1.Deployment
	Ollama    *appsv1.Deployment
}

// DeployServices deploys all custom K8s resources (non-Helm).
func DeployServices(ctx *pulumi.Context, cfg *InfraConfig) (*ServicesResult, error) {
	prefix := cfg.Prefix()
	ns := cfg.Namespace

	chromadb, err := deployChromaDB(ctx, prefix, ns)
	if err != nil {
		return nil, err
	}

	crawl4ai, err := deployCrawl4AI(ctx, prefix, ns)
	if err != nil {
		return nil, err
	}

	memos, err := deployMemos(ctx, prefix, ns)
	if err != nil {
		return nil, err
	}

	hindsight, err := deployHindsight(ctx, prefix, ns, cfg)
	if err != nil {
		return nil, err
	}

	mem0, err := deployMem0(ctx, prefix, ns, cfg)
	if err != nil {
		return nil, err
	}

	ollama, err := deployOllama(ctx, prefix, ns)
	if err != nil {
		return nil, err
	}

	return &ServicesResult{
		ChromaDB:  chromadb,
		Crawl4AI:  crawl4ai,
		Memos:     memos,
		Hindsight: hindsight,
		Mem0:      mem0,
		Ollama:    ollama,
	}, nil
}

// --- ChromaDB ---

func deployChromaDB(ctx *pulumi.Context, prefix, ns string) (*appsv1.Deployment, error) {
	name := fmt.Sprintf("%s-chromadb", prefix)
	labels := map[string]string{
		"app.kubernetes.io/name":      "chromadb",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "chromadb",
	}

	// PVC
	_, err := corev1.NewPersistentVolumeClaim(ctx, name, &corev1.PersistentVolumeClaimArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &corev1.PersistentVolumeClaimSpecArgs{
			AccessModes: pulumi.StringArray{pulumi.String("ReadWriteOnce")},
			Resources: &corev1.VolumeResourceRequirementsArgs{
				Requests: pulumi.StringMap{"storage": pulumi.String("1Gi")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// Deployment
	replicas := pulumi.Int(1)
	dep, err := appsv1.NewDeployment(ctx, name, &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Strategy: &appsv1.DeploymentStrategyArgs{
				Type: pulumi.String("Recreate"),
			},
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(labels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{
					Labels: pulumi.ToStringMap(labels),
				},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("chromadb"),
							Image:           pulumi.String("chromadb/chroma:latest"),
							ImagePullPolicy: pulumi.String("IfNotPresent"),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{
									Name:          pulumi.String("http"),
									ContainerPort: pulumi.Int(8000),
									Protocol:      pulumi.String("TCP"),
								},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{Name: pulumi.String("IS_PERSISTENT"), Value: pulumi.String("TRUE")},
								&corev1.EnvVarArgs{Name: pulumi.String("PERSIST_DIRECTORY"), Value: pulumi.String("/chroma/chroma")},
								&corev1.EnvVarArgs{Name: pulumi.String("ANONYMIZED_TELEMETRY"), Value: pulumi.String("FALSE")},
							},
							VolumeMounts: corev1.VolumeMountArray{
								&corev1.VolumeMountArgs{
									Name:      pulumi.String("data"),
									MountPath: pulumi.String("/chroma/chroma"),
								},
							},
							Resources: resourceRequirements("100m", "256Mi", "500m", "1Gi"),
						},
					},
					Volumes: corev1.VolumeArray{
						&corev1.VolumeArgs{
							Name: pulumi.String("data"),
							PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSourceArgs{
								ClaimName: pulumi.String(name),
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

	// Service
	_, err = corev1.NewService(ctx, name, &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(labels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{
					Name:       pulumi.String("http"),
					Port:       pulumi.Int(8000),
					TargetPort: pulumi.String("http"),
					Protocol:   pulumi.String("TCP"),
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return dep, nil
}

// --- Crawl4AI ---

func deployCrawl4AI(ctx *pulumi.Context, prefix, ns string) (*appsv1.Deployment, error) {
	name := fmt.Sprintf("%s-crawl4ai", prefix)
	labels := map[string]string{
		"app.kubernetes.io/name":      "crawl4ai",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "crawl4ai",
	}

	replicas := pulumi.Int(1)
	dep, err := appsv1.NewDeployment(ctx, name, &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(labels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{
					Labels: pulumi.ToStringMap(labels),
				},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("crawl4ai"),
							Image:           pulumi.String("unclecode/crawl4ai:latest"),
							ImagePullPolicy: pulumi.String("IfNotPresent"),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{
									Name:          pulumi.String("http"),
									ContainerPort: pulumi.Int(11235),
									Protocol:      pulumi.String("TCP"),
								},
							},
							Resources: resourceRequirements("100m", "256Mi", "1", "1Gi"),
						},
					},
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	_, err = corev1.NewService(ctx, name, &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(labels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{
					Name:       pulumi.String("http"),
					Port:       pulumi.Int(11235),
					TargetPort: pulumi.String("http"),
					Protocol:   pulumi.String("TCP"),
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return dep, nil
}

// --- Memos ---

func deployMemos(ctx *pulumi.Context, prefix, ns string) (*appsv1.Deployment, error) {
	pgName := fmt.Sprintf("%s-memos-pg", prefix)
	memosName := fmt.Sprintf("%s-memos", prefix)

	pgLabels := map[string]string{
		"app.kubernetes.io/name":      "memos-pg",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "memos-pg",
	}
	memosLabels := map[string]string{
		"app.kubernetes.io/name":      "memos",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "memos",
	}

	// PG PVC
	_, err := corev1.NewPersistentVolumeClaim(ctx, pgName, &corev1.PersistentVolumeClaimArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(pgName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(pgLabels),
		},
		Spec: &corev1.PersistentVolumeClaimSpecArgs{
			AccessModes: pulumi.StringArray{pulumi.String("ReadWriteOnce")},
			Resources: &corev1.VolumeResourceRequirementsArgs{
				Requests: pulumi.StringMap{"storage": pulumi.String("2Gi")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// PG Deployment
	replicas := pulumi.Int(1)
	_, err = appsv1.NewDeployment(ctx, pgName, &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(pgName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(pgLabels),
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Strategy: &appsv1.DeploymentStrategyArgs{Type: pulumi.String("Recreate")},
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(pgLabels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{Labels: pulumi.ToStringMap(pgLabels)},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("postgres"),
							Image:           pulumi.String("postgres:16-alpine"),
							ImagePullPolicy: pulumi.String("IfNotPresent"),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{Name: pulumi.String("postgres"), ContainerPort: pulumi.Int(5432), Protocol: pulumi.String("TCP")},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{Name: pulumi.String("POSTGRES_USER"), Value: pulumi.String("memos")},
								&corev1.EnvVarArgs{Name: pulumi.String("POSTGRES_PASSWORD"), Value: pulumi.String("memos")},
								&corev1.EnvVarArgs{Name: pulumi.String("POSTGRES_DB"), Value: pulumi.String("memos")},
								&corev1.EnvVarArgs{Name: pulumi.String("PGDATA"), Value: pulumi.String("/var/lib/postgresql/data/pgdata")},
							},
							VolumeMounts: corev1.VolumeMountArray{
								&corev1.VolumeMountArgs{Name: pulumi.String("data"), MountPath: pulumi.String("/var/lib/postgresql/data")},
							},
							Resources: resourceRequirements("50m", "64Mi", "250m", "256Mi"),
						},
					},
					Volumes: corev1.VolumeArray{
						&corev1.VolumeArgs{
							Name: pulumi.String("data"),
							PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSourceArgs{
								ClaimName: pulumi.String(pgName),
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

	// PG Service
	_, err = corev1.NewService(ctx, pgName, &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(pgName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(pgLabels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(pgLabels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{Name: pulumi.String("postgres"), Port: pulumi.Int(5432), TargetPort: pulumi.String("postgres"), Protocol: pulumi.String("TCP")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// Memos Deployment
	dep, err := appsv1.NewDeployment(ctx, memosName, &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(memosName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(memosLabels),
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(memosLabels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{Labels: pulumi.ToStringMap(memosLabels)},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("memos"),
							Image:           pulumi.String("neosmemo/memos:stable"),
							ImagePullPolicy: pulumi.String("IfNotPresent"),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{Name: pulumi.String("http"), ContainerPort: pulumi.Int(5230), Protocol: pulumi.String("TCP")},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{Name: pulumi.String("MEMOS_DRIVER"), Value: pulumi.String("postgres")},
								&corev1.EnvVarArgs{
									Name:  pulumi.String("MEMOS_DSN"),
									Value: pulumi.String(fmt.Sprintf("postgresql://memos:memos@%s:5432/memos?sslmode=disable", pgName)),
								},
							},
							Resources: resourceRequirements("50m", "64Mi", "250m", "256Mi"),
						},
					},
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// Memos Service
	_, err = corev1.NewService(ctx, memosName, &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(memosName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(memosLabels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(memosLabels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{Name: pulumi.String("http"), Port: pulumi.Int(5230), TargetPort: pulumi.String("http"), Protocol: pulumi.String("TCP")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return dep, nil
}

// --- Hindsight ---

func deployHindsight(ctx *pulumi.Context, prefix, ns string, cfg *InfraConfig) (*appsv1.Deployment, error) {
	pgName := fmt.Sprintf("%s-hindsight-pg", prefix)
	hindsightName := fmt.Sprintf("%s-hindsight", prefix)

	pgLabels := map[string]string{
		"app.kubernetes.io/name":      "hindsight-pg",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "hindsight-pg",
	}
	hindsightLabels := map[string]string{
		"app.kubernetes.io/name":      "hindsight",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "hindsight",
	}

	// PG PVC
	_, err := corev1.NewPersistentVolumeClaim(ctx, pgName, &corev1.PersistentVolumeClaimArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(pgName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(pgLabels),
		},
		Spec: &corev1.PersistentVolumeClaimSpecArgs{
			AccessModes: pulumi.StringArray{pulumi.String("ReadWriteOnce")},
			Resources: &corev1.VolumeResourceRequirementsArgs{
				Requests: pulumi.StringMap{"storage": pulumi.String("5Gi")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// PG Deployment
	replicas := pulumi.Int(1)
	_, err = appsv1.NewDeployment(ctx, pgName, &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(pgName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(pgLabels),
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Strategy: &appsv1.DeploymentStrategyArgs{Type: pulumi.String("Recreate")},
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(pgLabels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{Labels: pulumi.ToStringMap(pgLabels)},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("postgres"),
							Image:           pulumi.String("pgvector/pgvector:pg16"),
							ImagePullPolicy: pulumi.String("IfNotPresent"),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{Name: pulumi.String("postgres"), ContainerPort: pulumi.Int(5432), Protocol: pulumi.String("TCP")},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{Name: pulumi.String("POSTGRES_USER"), Value: pulumi.String("hindsight")},
								&corev1.EnvVarArgs{Name: pulumi.String("POSTGRES_PASSWORD"), Value: pulumi.String("hindsight")},
								&corev1.EnvVarArgs{Name: pulumi.String("POSTGRES_DB"), Value: pulumi.String("hindsight")},
								&corev1.EnvVarArgs{Name: pulumi.String("PGDATA"), Value: pulumi.String("/var/lib/postgresql/data/pgdata")},
							},
							VolumeMounts: corev1.VolumeMountArray{
								&corev1.VolumeMountArgs{Name: pulumi.String("data"), MountPath: pulumi.String("/var/lib/postgresql/data")},
							},
							Resources: resourceRequirements("50m", "64Mi", "250m", "256Mi"),
						},
					},
					Volumes: corev1.VolumeArray{
						&corev1.VolumeArgs{
							Name: pulumi.String("data"),
							PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSourceArgs{
								ClaimName: pulumi.String(pgName),
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

	// PG Service
	_, err = corev1.NewService(ctx, pgName, &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(pgName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(pgLabels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(pgLabels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{Name: pulumi.String("postgres"), Port: pulumi.Int(5432), TargetPort: pulumi.String("postgres"), Protocol: pulumi.String("TCP")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// Hindsight Deployment
	dep, err := appsv1.NewDeployment(ctx, hindsightName, &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(hindsightName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(hindsightLabels),
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(hindsightLabels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{Labels: pulumi.ToStringMap(hindsightLabels)},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("hindsight"),
							Image:           pulumi.String("ghcr.io/vectorize-io/hindsight:latest"),
							ImagePullPolicy: pulumi.String("IfNotPresent"),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{Name: pulumi.String("http"), ContainerPort: pulumi.Int(8888), Protocol: pulumi.String("TCP")},
								&corev1.ContainerPortArgs{Name: pulumi.String("admin"), ContainerPort: pulumi.Int(9999), Protocol: pulumi.String("TCP")},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{
									Name:  pulumi.String("HINDSIGHT_DB_URL"),
									Value: pulumi.String(fmt.Sprintf("postgresql://hindsight:hindsight@%s:5432/hindsight", pgName)),
								},
								&corev1.EnvVarArgs{Name: pulumi.String("HINDSIGHT_API_LLM_PROVIDER"), Value: pulumi.String(cfg.HindsightLLMProvider)},
								&corev1.EnvVarArgs{Name: pulumi.String("HINDSIGHT_API_LLM_MODEL"), Value: pulumi.String(cfg.HindsightLLMModel)},
								&corev1.EnvVarArgs{Name: pulumi.String("HINDSIGHT_API_LLM_BASE_URL"), Value: pulumi.String(cfg.HindsightLLMBaseURL)},
								&corev1.EnvVarArgs{Name: pulumi.String("HINDSIGHT_API_LLM_API_KEY"), Value: pulumi.String("")},
							},
							Resources: resourceRequirements("100m", "256Mi", "500m", "1Gi"),
						},
					},
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// Hindsight Service
	_, err = corev1.NewService(ctx, hindsightName, &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(hindsightName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(hindsightLabels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(hindsightLabels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{Name: pulumi.String("http"), Port: pulumi.Int(8888), TargetPort: pulumi.String("http"), Protocol: pulumi.String("TCP")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return dep, nil
}

// --- Mem0 ---

func deployMem0(ctx *pulumi.Context, prefix, ns string, cfg *InfraConfig) (*appsv1.Deployment, error) {
	name := fmt.Sprintf("%s-mem0", prefix)
	configMapName := fmt.Sprintf("%s-mem0-config", prefix)
	labels := map[string]string{
		"app.kubernetes.io/name":      "mem0",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "mem0",
	}

	// ConfigMap with patch_config.py
	patchScript := `import json
import os
import pathlib
import re


MAIN_PATH = pathlib.Path("/app/main.py")


def _env(name: str, default: str) -> str:
    value = os.environ.get(name)
    if value is None or not value.strip():
        return default
    return value.strip()


chroma_host = _env("CHROMA_HOST", "chromadb")
chroma_port = int(_env("CHROMA_PORT", "8000"))
history_db_path = _env("HISTORY_DB_PATH", "/tmp/mem0_history.db")
ollama_base_url = _env("MEM0_OLLAMA_BASE_URL", _env("OLLAMA_BASE_URL", "http://host.docker.internal:11434"))
llm_model = _env("MEM0_OLLAMA_LLM_MODEL", "llama3.2:latest")
embed_model = _env("MEM0_OLLAMA_EMBED_MODEL", "nomic-embed-text:latest")

config = {
    "version": "v1.1",
    "vector_store": {
        "provider": "chroma",
        "config": {
            "host": chroma_host,
            "port": chroma_port,
            "collection_name": "memories",
        },
    },
    "llm": {
        "provider": "ollama",
        "config": {
            "model": llm_model,
            "ollama_base_url": ollama_base_url,
            "temperature": 0.0,
            "max_tokens": 256,
        },
    },
    "embedder": {
        "provider": "ollama",
        "config": {
            "model": embed_model,
            "ollama_base_url": ollama_base_url,
        },
    },
    "history_db_path": history_db_path,
}

replacement = (
    "DEFAULT_CONFIG = " + json.dumps(config, indent=4) +
    "\n\n\ntry:\n"
    "    MEMORY_INSTANCE = Memory.from_config(DEFAULT_CONFIG)\n"
    "except Exception as e:\n"
    "    logging.exception('Mem0 startup initialization failed; service will start without active MEMORY_INSTANCE')\n"
    "    MEMORY_INSTANCE = None\n"
)

content = MAIN_PATH.read_text()
updated, count = re.subn(
    r"DEFAULT_CONFIG = \{.*?\nMEMORY_INSTANCE = Memory\.from_config\(DEFAULT_CONFIG\)",
    replacement,
    content,
    count=1,
    flags=re.S,
)
if count != 1:
    raise RuntimeError("Failed to patch mem0 main.py DEFAULT_CONFIG block")

MAIN_PATH.write_text(updated)
print("Patched /app/main.py for Chroma + Ollama")
print(f"  CHROMA_HOST={chroma_host}:{chroma_port}")
print(f"  OLLAMA_BASE_URL={ollama_base_url}")
print(f"  LLM_MODEL={llm_model}")
print(f"  EMBED_MODEL={embed_model}")
`

	_, err := corev1.NewConfigMap(ctx, configMapName, &corev1.ConfigMapArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(configMapName),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Data: pulumi.StringMap{
			"patch_config.py": pulumi.String(patchScript),
		},
	})
	if err != nil {
		return nil, err
	}

	// Deployment
	replicas := pulumi.Int(1)
	chromaDBName := fmt.Sprintf("%s-chromadb", prefix)
	dep, err := appsv1.NewDeployment(ctx, name, &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(labels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{Labels: pulumi.ToStringMap(labels)},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("mem0"),
							Image:           pulumi.String("mem0/mem0-api-server:latest"),
							ImagePullPolicy: pulumi.String("IfNotPresent"),
							Command:         pulumi.StringArray{pulumi.String("sh"), pulumi.String("-c")},
							Args: pulumi.StringArray{
								pulumi.String("pip install --quiet --no-cache-dir chromadb ollama && python3 /app/patch_config.py && exec uvicorn main:app --host 0.0.0.0 --port 8000 --workers 1"),
							},
							WorkingDir: pulumi.String("/app"),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{Name: pulumi.String("http"), ContainerPort: pulumi.Int(8000), Protocol: pulumi.String("TCP")},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{Name: pulumi.String("CHROMA_HOST"), Value: pulumi.String(chromaDBName)},
								&corev1.EnvVarArgs{Name: pulumi.String("CHROMA_PORT"), Value: pulumi.String("8000")},
								&corev1.EnvVarArgs{Name: pulumi.String("HISTORY_DB_PATH"), Value: pulumi.String("/tmp/mem0_history.db")},
								&corev1.EnvVarArgs{Name: pulumi.String("OLLAMA_BASE_URL"), Value: pulumi.String(cfg.Mem0OllamaBaseURL)},
								&corev1.EnvVarArgs{Name: pulumi.String("MEM0_OLLAMA_BASE_URL"), Value: pulumi.String(cfg.Mem0OllamaBaseURL)},
								&corev1.EnvVarArgs{Name: pulumi.String("MEM0_OLLAMA_LLM_MODEL"), Value: pulumi.String(cfg.Mem0OllamaModel)},
							},
							VolumeMounts: corev1.VolumeMountArray{
								&corev1.VolumeMountArgs{
									Name:      pulumi.String("patch-script"),
									MountPath: pulumi.String("/app/patch_config.py"),
									SubPath:   pulumi.String("patch_config.py"),
									ReadOnly:  pulumi.Bool(true),
								},
							},
							Resources: resourceRequirements("250m", "256Mi", "1", "1Gi"),
						},
					},
					Volumes: corev1.VolumeArray{
						&corev1.VolumeArgs{
							Name: pulumi.String("patch-script"),
							ConfigMap: &corev1.ConfigMapVolumeSourceArgs{
								Name: pulumi.String(configMapName),
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

	// Service
	_, err = corev1.NewService(ctx, name, &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(labels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{Name: pulumi.String("http"), Port: pulumi.Int(8000), TargetPort: pulumi.String("http"), Protocol: pulumi.String("TCP")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return dep, nil
}

// --- Ollama ---

func deployOllama(ctx *pulumi.Context, prefix, ns string) (*appsv1.Deployment, error) {
	name := fmt.Sprintf("%s-ollama", prefix)
	labels := map[string]string{
		"app.kubernetes.io/name":      "ollama",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "ollama",
	}

	// PVC
	_, err := corev1.NewPersistentVolumeClaim(ctx, name, &corev1.PersistentVolumeClaimArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &corev1.PersistentVolumeClaimSpecArgs{
			AccessModes: pulumi.StringArray{pulumi.String("ReadWriteOnce")},
			Resources: &corev1.VolumeResourceRequirementsArgs{
				Requests: pulumi.StringMap{"storage": pulumi.String("5Gi")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// Deployment
	replicas := pulumi.Int(1)
	dep, err := appsv1.NewDeployment(ctx, name, &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(labels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{Labels: pulumi.ToStringMap(labels)},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("ollama"),
							Image:           pulumi.String("ollama/ollama:latest"),
							ImagePullPolicy: pulumi.String("IfNotPresent"),
							Command:         pulumi.StringArray{pulumi.String("ollama"), pulumi.String("serve")},
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{Name: pulumi.String("http"), ContainerPort: pulumi.Int(11434), Protocol: pulumi.String("TCP")},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{Name: pulumi.String("OLLAMA_HOST"), Value: pulumi.String("0.0.0.0:11434")},
							},
							VolumeMounts: corev1.VolumeMountArray{
								&corev1.VolumeMountArgs{Name: pulumi.String("ollama-data"), MountPath: pulumi.String("/root/.ollama")},
							},
							Resources: resourceRequirements("100m", "256Mi", "1", "1Gi"),
						},
					},
					Volumes: corev1.VolumeArray{
						&corev1.VolumeArgs{
							Name: pulumi.String("ollama-data"),
							PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSourceArgs{
								ClaimName: pulumi.String(name),
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

	// Service
	_, err = corev1.NewService(ctx, name, &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(name),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(labels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(labels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{Name: pulumi.String("http"), Port: pulumi.Int(11434), TargetPort: pulumi.String("http"), Protocol: pulumi.String("TCP")},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return dep, nil
}

// resourceRequirements creates a resource requirements spec.
func resourceRequirements(cpuReq, memReq, cpuLim, memLim string) *corev1.ResourceRequirementsArgs {
	return &corev1.ResourceRequirementsArgs{
		Requests: pulumi.StringMap{
			"cpu":    pulumi.String(cpuReq),
			"memory": pulumi.String(memReq),
		},
		Limits: pulumi.StringMap{
			"cpu":    pulumi.String(cpuLim),
			"memory": pulumi.String(memLim),
		},
	}
}
