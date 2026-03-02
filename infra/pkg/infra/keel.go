package infra

import (
	"fmt"

	helmv4 "github.com/pulumi/pulumi-kubernetes/sdk/v4/go/kubernetes/helm/v4"
	"github.com/pulumi/pulumi/sdk/v3/go/pulumi"
)

// KeelResult holds references to the Keel deployment.
type KeelResult struct {
	Keel *helmv4.Chart
}

// DeployKeel deploys Keel image update controller via Helm.
func DeployKeel(ctx *pulumi.Context, cfg *InfraConfig) (*KeelResult, error) {
	name := fmt.Sprintf("%s-keel", cfg.Prefix())

	keel, err := helmv4.NewChart(ctx, name, &helmv4.ChartArgs{
		Chart:     pulumi.String("keel"),
		Namespace: pulumi.String(cfg.Namespace),
		RepositoryOpts: &helmv4.RepositoryOptsArgs{
			Repo: pulumi.String("https://charts.keel.sh"),
		},
		Values: pulumi.Map{
			"polling": pulumi.Map{
				"enabled":  pulumi.Bool(true),
				"schedule": pulumi.String("@every 2m"),
			},
			"webhook": pulumi.Map{
				"enabled": pulumi.Bool(false),
			},
			"basicauth": pulumi.Map{
				"enabled": pulumi.Bool(false),
			},
			"resources": pulumi.Map{
				"requests": pulumi.StringMap{
					"cpu":    pulumi.String("25m"),
					"memory": pulumi.String("64Mi"),
				},
				"limits": pulumi.StringMap{
					"cpu":    pulumi.String("100m"),
					"memory": pulumi.String("128Mi"),
				},
			},
		},
	})
	if err != nil {
		return nil, err
	}

	return &KeelResult{Keel: keel}, nil
}
