package infra

import (
	"fmt"

	helmv4 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/helm/v4"
	metav1 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/meta/v1"
	"github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/apiextensions"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// NetworkResult holds references to network layer resources.
type NetworkResult struct {
	Traefik     *helmv4.Chart
	CertManager *helmv4.Chart
}

// DeployNetwork deploys Traefik and cert-manager Helm charts,
// plus ClusterIssuer, CA Certificate, and wildcard Certificate resources.
func DeployNetwork(ctx *pulumi.Context, cfg *InfraConfig) (*NetworkResult, error) {
	prefix := cfg.Prefix()
	ns := cfg.Namespace

	// --- Traefik ---
	traefik, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-traefik", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("traefik"),
		Version: pulumi.String("39.0.2"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://traefik.github.io/charts"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"gateway": pulumi.Map{
				"enabled": pulumi.Bool(false),
			},
			"ingressRoute": pulumi.Map{
				"dashboard": pulumi.Map{
					"enabled": pulumi.Bool(false),
				},
			},
			"ports": pulumi.Map{
				"web": pulumi.Map{
					"http": pulumi.Map{
						"redirections": pulumi.Map{
							"entryPoint": pulumi.Map{
								"to":        pulumi.String("websecure"),
								"scheme":    pulumi.String("https"),
								"permanent": pulumi.Bool(true),
							},
						},
					},
				},
			},
			"additionalArguments": pulumi.StringArray{
				pulumi.String("--serversTransport.insecureSkipVerify=true"),
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// --- cert-manager ---
	certManager, err := helmv4.NewChart(ctx, fmt.Sprintf("%s-cert-manager", prefix), &helmv4.ChartArgs{
		Chart:   pulumi.String("cert-manager"),
		Version: pulumi.String("v1.19.3"),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://charts.jetstack.io"),
		},
		Namespace: pulumi.String(ns),
		Values: pulumi.Map{
			"crds": pulumi.Map{
				"enabled": pulumi.Bool(true),
				"keep":    pulumi.Bool(true),
			},
			"prometheus": pulumi.Map{
				"enabled": pulumi.Bool(true),
			},
		},
	})
	if err != nil {
		return nil, err
	}

	// --- Self-signed ClusterIssuer ---
	selfsignedIssuer, err := apiextensions.NewCustomResource(ctx, fmt.Sprintf("%s-selfsigned", prefix), &apiextensions.CustomResourceArgs{
		ApiVersion: pulumi.String("cert-manager.io/v1"),
		Kind:       pulumi.String("ClusterIssuer"),
		Metadata: &metav1.ObjectMetaArgs{
			Name: pulumi.String(fmt.Sprintf("%s-selfsigned", prefix)),
		},
		OtherFields: map[string]interface{}{
			"spec": map[string]interface{}{
				"selfSigned": map[string]interface{}{},
			},
		},
	}, pulumi.DependsOn([]pulumi.Resource{certManager}))
	if err != nil {
		return nil, err
	}

	// --- Root CA Certificate ---
	caCert, err := apiextensions.NewCustomResource(ctx, fmt.Sprintf("%s-ca", prefix), &apiextensions.CustomResourceArgs{
		ApiVersion: pulumi.String("cert-manager.io/v1"),
		Kind:       pulumi.String("Certificate"),
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-ca", prefix)),
			Namespace: pulumi.String(ns),
		},
		OtherFields: map[string]interface{}{
			"spec": map[string]interface{}{
				"isCA":       true,
				"commonName": fmt.Sprintf("%s CA", cfg.Domain),
				"secretName": fmt.Sprintf("%s-ca-tls", prefix),
				"duration":   "87600h",
				"renewBefore": "8760h",
				"privateKey": map[string]interface{}{
					"algorithm": "ECDSA",
					"size":      256,
				},
				"issuerRef": map[string]interface{}{
					"name":  fmt.Sprintf("%s-selfsigned", prefix),
					"kind":  "ClusterIssuer",
					"group": "cert-manager.io",
				},
			},
		},
	}, pulumi.DependsOn([]pulumi.Resource{selfsignedIssuer}))
	if err != nil {
		return nil, err
	}

	// --- CA ClusterIssuer ---
	caIssuer, err := apiextensions.NewCustomResource(ctx, fmt.Sprintf("%s-ca-issuer", prefix), &apiextensions.CustomResourceArgs{
		ApiVersion: pulumi.String("cert-manager.io/v1"),
		Kind:       pulumi.String("ClusterIssuer"),
		Metadata: &metav1.ObjectMetaArgs{
			Name: pulumi.String(fmt.Sprintf("%s-ca-issuer", prefix)),
		},
		OtherFields: map[string]interface{}{
			"spec": map[string]interface{}{
				"ca": map[string]interface{}{
					"secretName": fmt.Sprintf("%s-ca-tls", prefix),
				},
			},
		},
	}, pulumi.DependsOn([]pulumi.Resource{caCert}))
	if err != nil {
		return nil, err
	}

	// --- Wildcard Certificate ---
	_, err = apiextensions.NewCustomResource(ctx, fmt.Sprintf("%s-wildcard", prefix), &apiextensions.CustomResourceArgs{
		ApiVersion: pulumi.String("cert-manager.io/v1"),
		Kind:       pulumi.String("Certificate"),
		Metadata: &metav1.ObjectMetaArgs{
			Name:      pulumi.String(fmt.Sprintf("%s-wildcard", prefix)),
			Namespace: pulumi.String(ns),
		},
		OtherFields: map[string]interface{}{
			"spec": map[string]interface{}{
				"secretName":  fmt.Sprintf("%s-wildcard-tls", prefix),
				"duration":    "8760h",
				"renewBefore": "720h",
				"commonName":  fmt.Sprintf("*.%s", cfg.Domain),
				"dnsNames": []string{
					fmt.Sprintf("*.%s", cfg.Domain),
					cfg.Domain,
				},
				"privateKey": map[string]interface{}{
					"algorithm": "ECDSA",
					"size":      256,
				},
				"issuerRef": map[string]interface{}{
					"name":  fmt.Sprintf("%s-ca-issuer", prefix),
					"kind":  "ClusterIssuer",
					"group": "cert-manager.io",
				},
			},
		},
	}, pulumi.DependsOn([]pulumi.Resource{caIssuer}))
	if err != nil {
		return nil, err
	}

	return &NetworkResult{
		Traefik:     traefik,
		CertManager: certManager,
	}, nil
}
