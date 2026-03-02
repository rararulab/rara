package app

import (
	"fmt"

	appsv1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/apps/v1"
	corev1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/core/v1"
	metav1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/meta/v1"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// Run is the entry point for the app stack.
func Run(ctx *pulumi.Context) error {
	cfg := LoadAppConfig(ctx)
	prefix := cfg.Prefix()
	ns := cfg.Namespace

	// --- Backend ---
	backendLabels := map[string]string{
		"app.kubernetes.io/name":      "rara-app",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "backend",
	}

	backendImage := fmt.Sprintf("%s:%s", cfg.BackendImageRepo, cfg.BackendImageTag)
	replicas := pulumi.Int(1)

	_, err := appsv1.NewDeployment(ctx, fmt.Sprintf("%s-backend", prefix), &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-backend", prefix)),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(backendLabels),
			Annotations: pulumi.StringMap{
				"keel.sh/policy":       pulumi.String("force"),
				"keel.sh/trigger":      pulumi.String("poll"),
				"keel.sh/pollSchedule": pulumi.String("@every 2m"),
			},
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Strategy: &appsv1.DeploymentStrategyArgs{
				Type: pulumi.String("RollingUpdate"),
				RollingUpdate: &appsv1.RollingUpdateDeploymentArgs{
					MaxUnavailable: pulumi.String("0"),
					MaxSurge:       pulumi.String("1"),
				},
			},
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(backendLabels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{
					Labels: pulumi.ToStringMap(backendLabels),
				},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("backend"),
							Image:           pulumi.String(backendImage),
							ImagePullPolicy: pulumi.String(cfg.BackendPullPolicy),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{Name: pulumi.String("http"), ContainerPort: pulumi.Int(25555), Protocol: pulumi.String("TCP")},
								&corev1.ContainerPortArgs{Name: pulumi.String("grpc"), ContainerPort: pulumi.Int(50051), Protocol: pulumi.String("TCP")},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{Name: pulumi.String("CONSUL_HTTP_ADDR"), Value: pulumi.String(cfg.ConsulAddress)},
								&corev1.EnvVarArgs{Name: pulumi.String("RUST_LOG"), Value: pulumi.String("info,rara=debug")},
							},
							StartupProbe: &corev1.ProbeArgs{
								HttpGet: &corev1.HTTPGetActionArgs{
									Path: pulumi.String("/health"),
									Port: pulumi.String("http"),
								},
								InitialDelaySeconds: pulumi.Int(5),
								PeriodSeconds:       pulumi.Int(5),
								FailureThreshold:    pulumi.Int(12),
							},
							LivenessProbe: &corev1.ProbeArgs{
								HttpGet: &corev1.HTTPGetActionArgs{
									Path: pulumi.String("/api/v1/health"),
									Port: pulumi.String("http"),
								},
								InitialDelaySeconds: pulumi.Int(15),
								PeriodSeconds:       pulumi.Int(15),
							},
							ReadinessProbe: &corev1.ProbeArgs{
								HttpGet: &corev1.HTTPGetActionArgs{
									Path: pulumi.String("/api/v1/health"),
									Port: pulumi.String("http"),
								},
								InitialDelaySeconds: pulumi.Int(10),
								PeriodSeconds:       pulumi.Int(10),
							},
							Resources: &corev1.ResourceRequirementsArgs{
								Requests: pulumi.StringMap{
									"cpu":    pulumi.String("100m"),
									"memory": pulumi.String("256Mi"),
								},
								Limits: pulumi.StringMap{
									"cpu":    pulumi.String("1"),
									"memory": pulumi.String("1Gi"),
								},
							},
						},
					},
				},
			},
		},
	})
	if err != nil {
		return err
	}

	// Backend Service
	_, err = corev1.NewService(ctx, fmt.Sprintf("%s-backend", prefix), &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-backend", prefix)),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(backendLabels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(backendLabels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{Name: pulumi.String("http"), Port: pulumi.Int(25555), TargetPort: pulumi.String("http"), Protocol: pulumi.String("TCP")},
				&corev1.ServicePortArgs{Name: pulumi.String("grpc"), Port: pulumi.Int(50051), TargetPort: pulumi.String("grpc"), Protocol: pulumi.String("TCP")},
			},
		},
	})
	if err != nil {
		return err
	}

	// --- Frontend ---
	frontendLabels := map[string]string{
		"app.kubernetes.io/name":      "rara-app",
		"app.kubernetes.io/instance":  prefix,
		"app.kubernetes.io/component": "frontend",
	}

	frontendImage := fmt.Sprintf("%s:%s", cfg.FrontendImageRepo, cfg.FrontendImageTag)

	_, err = appsv1.NewDeployment(ctx, fmt.Sprintf("%s-frontend", prefix), &appsv1.DeploymentArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-frontend", prefix)),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(frontendLabels),
			Annotations: pulumi.StringMap{
				"keel.sh/policy":       pulumi.String("force"),
				"keel.sh/trigger":      pulumi.String("poll"),
				"keel.sh/pollSchedule": pulumi.String("@every 2m"),
			},
		},
		Spec: &appsv1.DeploymentSpecArgs{
			Replicas: replicas,
			Selector: &metav1.LabelSelectorArgs{
				MatchLabels: pulumi.ToStringMap(frontendLabels),
			},
			Template: &corev1.PodTemplateSpecArgs{
				Metadata: &metav1.ObjectMetaArgs{
					Labels: pulumi.ToStringMap(frontendLabels),
				},
				Spec: &corev1.PodSpecArgs{
					Containers: corev1.ContainerArray{
						&corev1.ContainerArgs{
							Name:            pulumi.String("frontend"),
							Image:           pulumi.String(frontendImage),
							ImagePullPolicy: pulumi.String(cfg.FrontendPullPolicy),
							Ports: corev1.ContainerPortArray{
								&corev1.ContainerPortArgs{Name: pulumi.String("http"), ContainerPort: pulumi.Int(80), Protocol: pulumi.String("TCP")},
							},
							Env: corev1.EnvVarArray{
								&corev1.EnvVarArgs{
									Name:  pulumi.String("API_URL"),
									Value: pulumi.String(fmt.Sprintf("http://%s-backend:25555", prefix)),
								},
							},
							LivenessProbe: &corev1.ProbeArgs{
								HttpGet: &corev1.HTTPGetActionArgs{
									Path: pulumi.String("/"),
									Port: pulumi.String("http"),
								},
								InitialDelaySeconds: pulumi.Int(5),
							},
							ReadinessProbe: &corev1.ProbeArgs{
								HttpGet: &corev1.HTTPGetActionArgs{
									Path: pulumi.String("/"),
									Port: pulumi.String("http"),
								},
								InitialDelaySeconds: pulumi.Int(3),
							},
							Resources: &corev1.ResourceRequirementsArgs{
								Requests: pulumi.StringMap{
									"cpu":    pulumi.String("25m"),
									"memory": pulumi.String("32Mi"),
								},
								Limits: pulumi.StringMap{
									"cpu":    pulumi.String("100m"),
									"memory": pulumi.String("128Mi"),
								},
							},
						},
					},
				},
			},
		},
	})
	if err != nil {
		return err
	}

	// Frontend Service
	_, err = corev1.NewService(ctx, fmt.Sprintf("%s-frontend", prefix), &corev1.ServiceArgs{
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-frontend", prefix)),
			Namespace: pulumi.String(ns),
			Labels:    pulumi.ToStringMap(frontendLabels),
		},
		Spec: &corev1.ServiceSpecArgs{
			Type:     pulumi.String("ClusterIP"),
			Selector: pulumi.ToStringMap(frontendLabels),
			Ports: corev1.ServicePortArray{
				&corev1.ServicePortArgs{Name: pulumi.String("http"), Port: pulumi.Int(80), TargetPort: pulumi.String("http"), Protocol: pulumi.String("TCP")},
			},
		},
	})
	if err != nil {
		return err
	}

	// --- IngressRoutes + ServiceMonitor ---
	if err := DeployIngress(ctx, cfg); err != nil {
		return err
	}

	// Exports
	ctx.Export("backendService", pulumi.Sprintf("%s-backend.%s.svc.cluster.local:25555", prefix, ns))
	ctx.Export("frontendService", pulumi.Sprintf("%s-frontend.%s.svc.cluster.local:80", prefix, ns))
	ctx.Export("appUrl", pulumi.Sprintf("https://app.%s", cfg.Domain))
	ctx.Export("apiUrl", pulumi.Sprintf("https://api.%s", cfg.Domain))

	return nil
}
