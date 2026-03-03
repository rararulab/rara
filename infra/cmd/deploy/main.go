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
	"context"
	"fmt"
	"os"

	"github.com/urfave/cli/v2"
	"go.uber.org/zap"

	"github.com/rararulab/rara/infra/pkg/deploy"
	"github.com/rararulab/rara/infra/pkg/doctor"
)

func main() {
	app := &cli.App{
		Name:  "deploy",
		Usage: "Build, push, and deploy rara images to K8s",
		Commands: []*cli.Command{
			// --- build ---
			{
				Name:  "build",
				Usage: "Build Docker images locally",
				Subcommands: []*cli.Command{
					{
						Name:  "base",
						Usage: "Build the Rust base image (run once or after toolchain change)",
						Action: func(c *cli.Context) error {
							if err := deploy.ChdirToGitRoot(); err != nil {
								return err
							}
							return deploy.Build("docker/base.Dockerfile", []string{"rara-base:latest"}, nil)
						},
					},
					{
						Name:  "backend",
						Usage: "Build the backend image (requires base image)",
						Action: func(c *cli.Context) error {
							if err := deploy.ChdirToGitRoot(); err != nil {
								return err
							}
							return deploy.Build("docker/Dockerfile", []string{"rara:latest"}, nil)
						},
					},
					{
						Name:  "frontend",
						Usage: "Build the frontend image",
						Action: func(c *cli.Context) error {
							if err := deploy.ChdirToGitRoot(); err != nil {
								return err
							}
							return deploy.Build("docker/web.Dockerfile", []string{"ghcr.io/rararulab/rara-web:latest"}, nil)
						},
					},
				},
			},
			// --- deploy ---
			{
				Name:  "backend",
				Usage: "Deploy the backend to K8s",
				Action: func(c *cli.Context) error {
					return runDeploy(c.Context, deploy.Backend)
				},
			},
			{
				Name:  "frontend",
				Usage: "Deploy the frontend to K8s",
				Action: func(c *cli.Context) error {
					return runDeploy(c.Context, deploy.Frontend)
				},
			},
			{
				Name:  "all",
				Usage: "Deploy backend and frontend to K8s",
				Action: func(c *cli.Context) error {
					if err := runDeploy(c.Context, deploy.Backend); err != nil {
						return err
					}
					return runDeploy(c.Context, deploy.Frontend)
				},
			},
			// --- doctor ---
			{
				Name:  "doctor",
				Usage: "Check infrastructure health across all components",
				Flags: []cli.Flag{
					&cli.StringFlag{
						Name:    "namespace",
						Aliases: []string{"n"},
						Value:   "rara",
						Usage:   "Kubernetes namespace",
					},
					&cli.StringFlag{
						Name:    "release",
						Aliases: []string{"r"},
						Value:   "rara-infra",
						Usage:   "Helm release name",
					},
					&cli.StringFlag{
						Name:    "domain",
						Aliases: []string{"d"},
						Value:   "",
						Usage:   "Domain suffix (default: from Helm values or rara.local)",
					},
				},
				Action: func(c *cli.Context) error {
					kube, err := deploy.NewKubeClient()
					if err != nil {
						return fmt.Errorf("init kube client: %w", err)
					}

					cfg := doctor.Config{
						Namespace: c.String("namespace"),
						Release:   c.String("release"),
						Domain:    c.String("domain"),
					}

					report, err := doctor.Run(c.Context, kube.Clientset(), kube.RESTConfig(), cfg)
					if err != nil {
						return err
					}

					doctor.NewPrinter(os.Stdout).PrintReport(report)

					if report.HasFailures() {
						os.Exit(1)
					}
					return nil
				},
			},
		},
	}

	if err := app.Run(os.Args); err != nil {
		fmt.Fprintf(os.Stderr, "❌ %v\n", err)
		os.Exit(1)
	}
}

func runDeploy(ctx context.Context, target deploy.Target) error {
	cfg := zap.NewDevelopmentConfig()
	cfg.DisableStacktrace = true
	log, err := cfg.Build()
	if err != nil {
		return fmt.Errorf("init logger: %w", err)
	}
	defer log.Sync()

	if err := deploy.ChdirToGitRoot(); err != nil {
		return err
	}

	kube, err := deploy.NewKubeClient()
	if err != nil {
		return fmt.Errorf("init kube client: %w", err)
	}

	return deploy.Deploy(ctx, log, kube, target)
}
