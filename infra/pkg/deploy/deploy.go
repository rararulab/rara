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

package deploy

import (
	"context"
	"fmt"
	"time"

	"go.uber.org/zap"
)

// Target defines a deployable component.
type Target struct {
	Name       string
	Image      string            // GHCR image without tag
	Dockerfile string            // path to Dockerfile
	BuildArgs  map[string]string // docker build args
	Deployment string            // K8s deployment name
	Container  string            // K8s container name
	Namespace  string            // K8s namespace
	Timeout    time.Duration     // rollout timeout
	PreBuild   func() error      // optional pre-build step
}

var (
	Backend = Target{
		Name:       "backend",
		Image:      "ghcr.io/rararulab/rara",
		Dockerfile: "docker/Dockerfile",
		Deployment: "rara-app-backend",
		Container:  "backend",
		Namespace:  "rara",
		Timeout:    120 * time.Second,
		PreBuild: func() error {
			return Build("docker/base.Dockerfile", []string{"rara-base:latest"}, nil)
		},
	}

	Frontend = Target{
		Name:       "frontend",
		Image:      "ghcr.io/rararulab/rara-web",
		Dockerfile: "docker/web.Dockerfile",
		Deployment: "rara-app-frontend",
		Container:  "frontend",
		Namespace:  "rara",
		Timeout:    60 * time.Second,
	}
)

// Deploy executes the build-push-update flow for a target.
func Deploy(ctx context.Context, log *zap.Logger, kube *KubeClient, t Target) error {
	log = log.With(zap.String("target", t.Name))

	sha, err := GitSHA()
	if err != nil {
		return fmt.Errorf("git SHA: %w", err)
	}

	dirty, err := GitIsDirty()
	if err != nil {
		return fmt.Errorf("git dirty check: %w", err)
	}

	if dirty {
		log.Warn("working tree is dirty, forcing rebuild")
		sha += "-dirty"
	} else {
		ref := fmt.Sprintf("%s:sha-%s", t.Image, sha)
		if ImageExists(ref) {
			log.Info("image already exists in GHCR", zap.String("ref", ref))
			current, err := kube.CurrentImage(ctx, t.Namespace, t.Deployment, t.Container)
			if err == nil && current == ref {
				log.Info("K8s already running this image, nothing to do", zap.String("sha", sha))
				return nil
			}
			log.Info("K8s running different image, updating", zap.String("current", current), zap.String("new", ref))
			if err := kube.SetImage(ctx, t.Namespace, t.Deployment, t.Container, ref); err != nil {
				return fmt.Errorf("set image: %w", err)
			}
			if err := kube.WaitRollout(ctx, t.Namespace, t.Deployment, t.Timeout); err != nil {
				return fmt.Errorf("rollout: %w", err)
			}
			log.Info("updated", zap.String("sha", sha))
			return nil
		}
	}

	// Build
	if t.PreBuild != nil {
		log.Info("running pre-build step")
		if err := t.PreBuild(); err != nil {
			return fmt.Errorf("pre-build: %w", err)
		}
	}

	imageTag := fmt.Sprintf("%s:sha-%s", t.Image, sha)
	imageLatest := t.Image + ":latest"
	log.Info("building image", zap.String("tag", imageTag))
	if err := Build(t.Dockerfile, []string{imageTag, imageLatest}, t.BuildArgs); err != nil {
		return fmt.Errorf("docker build: %w", err)
	}

	log.Info("pushing to GHCR")
	if err := Push(imageTag); err != nil {
		return fmt.Errorf("push tag: %w", err)
	}
	if err := Push(imageLatest); err != nil {
		return fmt.Errorf("push latest: %w", err)
	}

	log.Info("updating K8s deployment")
	if err := kube.SetImage(ctx, t.Namespace, t.Deployment, t.Container, imageTag); err != nil {
		return fmt.Errorf("set image: %w", err)
	}
	if err := kube.WaitRollout(ctx, t.Namespace, t.Deployment, t.Timeout); err != nil {
		return fmt.Errorf("rollout: %w", err)
	}

	log.Info("deployed", zap.String("sha", sha))
	return nil
}
