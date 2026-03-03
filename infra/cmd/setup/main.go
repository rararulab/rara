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

package main

import (
	"fmt"
	"os"

	"github.com/urfave/cli/v2"
	"k8s.io/client-go/tools/clientcmd"

	"github.com/rararulab/rara/infra/pkg/setup"
)

func main() {
	app := &cli.App{
		Name:  "setup",
		Usage: "Spin up a complete rara local environment using kind",
		Commands: []*cli.Command{
			upCmd(),
			downCmd(),
			statusCmd(),
			hostsCmd(),
			seedCmd(),
		},
	}

	if err := app.Run(os.Args); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

// configFromCtx builds a setup.Config from CLI context.
func configFromCtx(c *cli.Context) setup.Config {
	cfg := setup.DefaultConfig()
	if v := c.String("cluster"); v != "" {
		cfg.ClusterName = v
	}
	if v := c.String("namespace"); v != "" {
		cfg.Namespace = v
	}
	if v := c.String("domain"); v != "" {
		cfg.Domain = v
	}
	if v := c.String("infra-release"); v != "" {
		cfg.InfraRelease = v
	}
	if v := c.String("postgres-password"); v != "" {
		cfg.PostgresPassword = v
	}
	if v := c.String("postgres-database"); v != "" {
		cfg.PostgresDatabase = v
	}
	if v := c.String("minio-user"); v != "" {
		cfg.MinioUser = v
	}
	if v := c.String("minio-password"); v != "" {
		cfg.MinioPassword = v
	}
	if v := c.String("langfuse-public-key"); v != "" {
		cfg.LangfusePublicKey = v
	}
	if v := c.String("langfuse-secret-key"); v != "" {
		cfg.LangfuseSecretKey = v
	}
	cfg.EnableOllama = !c.Bool("no-ollama")
	cfg.EnableMemos = !c.Bool("no-memos")
	cfg.EnableHindsight = !c.Bool("no-hindsight")
	cfg.EnableMem0 = !c.Bool("no-mem0")
	return cfg
}

// commonFlags returns the shared config flags.
func commonFlags() []cli.Flag {
	defs := setup.DefaultConfig()
	return []cli.Flag{
		&cli.StringFlag{
			Name:  "cluster",
			Value: defs.ClusterName,
			Usage: "kind cluster name",
		},
		&cli.StringFlag{
			Name:    "namespace",
			Aliases: []string{"n"},
			Value:   defs.Namespace,
			Usage:   "Kubernetes namespace",
		},
		&cli.StringFlag{
			Name:  "domain",
			Value: defs.Domain,
			Usage: "local domain suffix (e.g. rara.local)",
		},
		&cli.StringFlag{
			Name:  "infra-release",
			Value: defs.InfraRelease,
			Usage: "Helm release prefix for infra stack",
		},
	}
}

func upCmd() *cli.Command {
	defs := setup.DefaultConfig()
	return &cli.Command{
		Name:  "up",
		Usage: "Create kind cluster and deploy all rara infrastructure",
		Flags: append(commonFlags(),
			&cli.StringFlag{Name: "postgres-password", Value: defs.PostgresPassword, Usage: "PostgreSQL admin password"},
			&cli.StringFlag{Name: "postgres-database", Value: defs.PostgresDatabase, Usage: "PostgreSQL database name"},
			&cli.StringFlag{Name: "minio-user", Value: defs.MinioUser, Usage: "MinIO root user"},
			&cli.StringFlag{Name: "minio-password", Value: defs.MinioPassword, Usage: "MinIO root password"},
			&cli.StringFlag{Name: "langfuse-public-key", Value: "", Usage: "Langfuse public key (optional)"},
			&cli.StringFlag{Name: "langfuse-secret-key", Value: "", Usage: "Langfuse secret key (optional)"},
			&cli.BoolFlag{Name: "no-ollama", Value: false, Usage: "Skip Ollama deployment"},
			&cli.BoolFlag{Name: "no-memos", Value: false, Usage: "Skip Memos deployment"},
			&cli.BoolFlag{Name: "no-hindsight", Value: false, Usage: "Skip Hindsight deployment"},
			&cli.BoolFlag{Name: "no-mem0", Value: false, Usage: "Skip Mem0 deployment"},
		),
		Action: func(c *cli.Context) error {
			cfg := configFromCtx(c)
			return setup.Up(c.Context, cfg)
		},
	}
}

func downCmd() *cli.Command {
	return &cli.Command{
		Name:  "down",
		Usage: "Delete the kind cluster and clean up /etc/hosts",
		Flags: commonFlags(),
		Action: func(c *cli.Context) error {
			cfg := configFromCtx(c)
			return setup.Down(cfg)
		},
	}
}

func statusCmd() *cli.Command {
	return &cli.Command{
		Name:  "status",
		Usage: "Show the status of the local rara environment",
		Flags: commonFlags(),
		Action: func(c *cli.Context) error {
			cfg := configFromCtx(c)
			return setup.Status(c.Context, cfg)
		},
	}
}

func hostsCmd() *cli.Command {
	return &cli.Command{
		Name:  "hosts",
		Usage: "Manage /etc/hosts entries for rara services",
		Subcommands: []*cli.Command{
			{
				Name:  "add",
				Usage: "Add /etc/hosts entries for the rara domain (requires root)",
				Flags: append(commonFlags(),
					&cli.StringFlag{
						Name:  "ip",
						Value: "",
						Usage: "Override the LoadBalancer IP (auto-detected by default)",
					},
				),
				Action: func(c *cli.Context) error {
					cfg := configFromCtx(c)
					ip := c.String("ip")
					if ip == "" {
						kubeconfigPath := setup.KindKubeconfigPath(cfg.ClusterName)
						rc, err := clientcmd.BuildConfigFromFlags("", kubeconfigPath)
						if err != nil {
							return fmt.Errorf("build rest config: %w", err)
						}
						ip, err = setup.GetTraefikIP(c.Context, rc, cfg)
						if err != nil {
							return fmt.Errorf("auto-detect LoadBalancer IP: %w", err)
						}
						setup.Info(fmt.Sprintf("Detected LoadBalancer IP: %s", ip))
					}
					return setup.AddHostsEntries(ip, cfg.Domain)
				},
			},
			{
				Name:  "remove",
				Usage: "Remove rara /etc/hosts entries (requires root)",
				Action: func(c *cli.Context) error {
					return setup.RemoveHostsEntries()
				},
			},
			{
				Name:  "show",
				Usage: "Print what would be added to /etc/hosts",
				Flags: append(commonFlags(),
					&cli.StringFlag{Name: "ip", Value: "127.0.0.1", Usage: "IP address to use"},
				),
				Action: func(c *cli.Context) error {
					cfg := configFromCtx(c)
					setup.PrintHostsBlock(c.String("ip"), cfg.Domain)
					return nil
				},
			},
		},
	}
}

func seedCmd() *cli.Command {
	defs := setup.DefaultConfig()
	return &cli.Command{
		Name:  "seed",
		Usage: "Seed Consul KV with rara configuration values",
		Flags: append(commonFlags(),
			&cli.StringFlag{Name: "postgres-password", Value: defs.PostgresPassword, Usage: "PostgreSQL admin password"},
			&cli.StringFlag{Name: "postgres-database", Value: defs.PostgresDatabase, Usage: "PostgreSQL database name"},
			&cli.StringFlag{Name: "minio-user", Value: defs.MinioUser, Usage: "MinIO root user"},
			&cli.StringFlag{Name: "minio-password", Value: defs.MinioPassword, Usage: "MinIO root password"},
			&cli.StringFlag{Name: "langfuse-public-key", Value: "", Usage: "Langfuse public key (optional)"},
			&cli.StringFlag{Name: "langfuse-secret-key", Value: "", Usage: "Langfuse secret key (optional)"},
		),
		Action: func(c *cli.Context) error {
			cfg := configFromCtx(c)
			kubeconfigPath := setup.KindKubeconfigPath(cfg.ClusterName)
			return setup.SeedConsulKV(c.Context, cfg, kubeconfigPath)
		},
	}
}
