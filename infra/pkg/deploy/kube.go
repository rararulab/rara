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
	"path/filepath"
	"time"

	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
	"k8s.io/client-go/util/homedir"
	"k8s.io/client-go/util/retry"
)

// KubeClient wraps a Kubernetes clientset and its REST config.
type KubeClient struct {
	clientset  *kubernetes.Clientset
	restConfig *rest.Config
}

// NewKubeClient creates a Kubernetes client, trying in-cluster first, then ~/.kube/config.
func NewKubeClient() (*KubeClient, error) {
	config, err := rest.InClusterConfig()
	if err != nil {
		kubeconfig := filepath.Join(homedir.HomeDir(), ".kube", "config")
		config, err = clientcmd.BuildConfigFromFlags("", kubeconfig)
		if err != nil {
			return nil, fmt.Errorf("build kubeconfig: %w", err)
		}
	}

	clientset, err := kubernetes.NewForConfig(config)
	if err != nil {
		return nil, fmt.Errorf("create clientset: %w", err)
	}

	return &KubeClient{clientset: clientset, restConfig: config}, nil
}

// Clientset returns the underlying Kubernetes clientset.
func (k *KubeClient) Clientset() *kubernetes.Clientset {
	return k.clientset
}

// RESTConfig returns the underlying REST config.
func (k *KubeClient) RESTConfig() *rest.Config {
	return k.restConfig
}

// CurrentImage returns the image of a named container in a deployment.
func (k *KubeClient) CurrentImage(ctx context.Context, namespace, deployment, container string) (string, error) {
	deploy, err := k.clientset.AppsV1().Deployments(namespace).Get(ctx, deployment, metav1.GetOptions{})
	if err != nil {
		return "", err
	}

	for _, c := range deploy.Spec.Template.Spec.Containers {
		if c.Name == container {
			return c.Image, nil
		}
	}
	return "", fmt.Errorf("container %q not found in deployment %s", container, deployment)
}

// SetImage updates the container image on a deployment with conflict retry.
func (k *KubeClient) SetImage(ctx context.Context, namespace, deployment, container, image string) error {
	deploymentsClient := k.clientset.AppsV1().Deployments(namespace)

	return retry.RetryOnConflict(retry.DefaultRetry, func() error {
		deploy, err := deploymentsClient.Get(ctx, deployment, metav1.GetOptions{})
		if err != nil {
			return err
		}

		for i := range deploy.Spec.Template.Spec.Containers {
			if deploy.Spec.Template.Spec.Containers[i].Name == container {
				deploy.Spec.Template.Spec.Containers[i].Image = image
				break
			}
		}

		_, err = deploymentsClient.Update(ctx, deploy, metav1.UpdateOptions{})
		return err
	})
}

// WaitRollout polls the deployment until the rollout is complete or the timeout expires.
func (k *KubeClient) WaitRollout(ctx context.Context, namespace, deployment string, timeout time.Duration) error {
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return fmt.Errorf("timeout waiting for rollout of %s", deployment)
		case <-ticker.C:
			deploy, err := k.clientset.AppsV1().Deployments(namespace).Get(ctx, deployment, metav1.GetOptions{})
			if err != nil {
				return err
			}
			if isRolloutComplete(deploy) {
				return nil
			}
		}
	}
}

func isRolloutComplete(deploy *appsv1.Deployment) bool {
	if deploy.Spec.Replicas == nil {
		return false
	}
	desired := *deploy.Spec.Replicas
	if deploy.Status.UpdatedReplicas != desired {
		return false
	}
	if deploy.Status.AvailableReplicas != desired {
		return false
	}
	if deploy.Status.ObservedGeneration < deploy.Generation {
		return false
	}
	for _, cond := range deploy.Status.Conditions {
		if cond.Type == appsv1.DeploymentProgressing {
			return cond.Status == corev1.ConditionTrue && cond.Reason == "NewReplicaSetAvailable"
		}
	}
	return false
}
