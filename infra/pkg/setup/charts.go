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

package setup

import (
	"context"
	"fmt"
	"strings"
	"time"

	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	k8serrors "k8s.io/apimachinery/pkg/api/errors"
	"k8s.io/apimachinery/pkg/api/resource"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/util/intstr"
	"k8s.io/apimachinery/pkg/util/wait"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
)

// InstallHelmCharts installs all infrastructure Helm charts in order.
func InstallHelmCharts(ctx context.Context, cfg Config, kubeconfigPath string, send Sender) error {
	helm := NewHelmManager(kubeconfigPath, cfg.Namespace)
	prefix := cfg.Prefix()
	ns := cfg.Namespace

	// --- cert-manager ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing cert-manager (%s-cert-manager)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-cert-manager", prefix),
		"cert-manager",
		"v1.19.3",
		"https://charts.jetstack.io",
		map[string]interface{}{
			"crds": map[string]interface{}{
				"enabled": true,
				"keep":    true,
			},
			"prometheus": map[string]interface{}{
				"enabled": true,
			},
		},
	); err != nil {
		return err
	}

	// --- Traefik ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing Traefik (%s-traefik)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-traefik", prefix),
		"traefik",
		"39.0.2",
		"https://traefik.github.io/charts",
		map[string]interface{}{
			"gateway": map[string]interface{}{"enabled": false},
			"ingressRoute": map[string]interface{}{
				"dashboard": map[string]interface{}{"enabled": false},
			},
			"additionalArguments": []interface{}{
				"--serversTransport.insecureSkipVerify=true",
			},
		},
	); err != nil {
		return err
	}

	// --- PostgreSQL ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing PostgreSQL (%s-postgresql)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-postgresql", prefix),
		"postgresql",
		"18.4.0",
		"https://charts.bitnami.com/bitnami",
		map[string]interface{}{
			"global": map[string]interface{}{
				"security": map[string]interface{}{
					"allowInsecureImages": true,
				},
			},
			"image": map[string]interface{}{
				"registry":   "ghcr.io",
				"repository": "pgmq/pg18-pgmq",
				"tag":        "v1.10.0",
			},
			"auth": map[string]interface{}{
				"postgresPassword": cfg.PostgresPassword,
				"database":         cfg.PostgresDatabase,
				"usePasswordFiles": false,
			},
			"volumePermissions": map[string]interface{}{"enabled": true},
			"primary": map[string]interface{}{
				"podSecurityContext": map[string]interface{}{
					"enabled": true,
					"fsGroup": 999,
				},
				"containerSecurityContext": map[string]interface{}{
					"enabled":                true,
					"runAsUser":              999,
					"runAsGroup":             999,
					"readOnlyRootFilesystem": false,
				},
				"persistence": map[string]interface{}{
					"enabled": true,
					"size":    "2Gi",
				},
				"resources": map[string]interface{}{
					"requests": map[string]interface{}{"cpu": "100m", "memory": "256Mi"},
					"limits":   map[string]interface{}{"cpu": "500m", "memory": "512Mi"},
				},
			},
		},
	); err != nil {
		return err
	}

	// --- MinIO ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing MinIO (%s-minio)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-minio", prefix),
		"minio",
		"5.4.0",
		"https://charts.min.io",
		map[string]interface{}{
			"mode":         "standalone",
			"rootUser":     cfg.MinioUser,
			"rootPassword": cfg.MinioPassword,
			"consoleService": map[string]interface{}{
				"type": "ClusterIP",
				"port": 9001,
			},
			"service": map[string]interface{}{
				"type": "ClusterIP",
				"port": 9000,
			},
			"buckets": []interface{}{
				map[string]interface{}{"name": "rara", "policy": "none", "purge": false},
				map[string]interface{}{"name": "langfuse", "policy": "none", "purge": false},
				map[string]interface{}{"name": "quickwit", "policy": "none", "purge": false},
			},
			"persistence": map[string]interface{}{
				"enabled": true,
				"size":    "2Gi",
			},
			"resources": map[string]interface{}{
				"requests": map[string]interface{}{"cpu": "100m", "memory": "512Mi"},
				"limits":   map[string]interface{}{"cpu": "1", "memory": "1Gi"},
			},
			"makeBucketJob": map[string]interface{}{
				"backoffLimit": 30,
			},
		},
	); err != nil {
		return err
	}

	// --- Consul ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing Consul (%s-consul)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-consul", prefix),
		"consul",
		"1.9.3",
		"https://helm.releases.hashicorp.com",
		map[string]interface{}{
			"global": map[string]interface{}{
				"name":       "consul",
				"datacenter": "rara-dc1",
			},
			"server": map[string]interface{}{
				"replicas": 1,
				"storage":  "1Gi",
				"resources": map[string]interface{}{
					"requests": map[string]interface{}{"cpu": "100m", "memory": "128Mi"},
					"limits":   map[string]interface{}{"cpu": "500m", "memory": "256Mi"},
				},
			},
			"client":        map[string]interface{}{"enabled": true},
			"ui":            map[string]interface{}{"enabled": true},
			"connectInject": map[string]interface{}{"enabled": false},
		},
	); err != nil {
		return err
	}

	// --- kube-prometheus-stack ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing kube-prometheus-stack (%s-kube-prometheus-stack)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-kube-prometheus-stack", prefix),
		"kube-prometheus-stack",
		"82.2.1",
		"https://prometheus-community.github.io/helm-charts",
		map[string]interface{}{
			"grafana": map[string]interface{}{
				"adminPassword": "admin",
				"persistence":   map[string]interface{}{"enabled": true, "size": "1Gi"},
			},
			"prometheus": map[string]interface{}{
				"prometheusSpec": map[string]interface{}{
					"retention":              "3d",
					"storageSpec":            map[string]interface{}{"volumeClaimTemplate": map[string]interface{}{"spec": map[string]interface{}{"resources": map[string]interface{}{"requests": map[string]interface{}{"storage": "2Gi"}}}}},
					"serviceMonitorSelectorNilUsesHelmValues": false,
				},
			},
			"alertmanager": map[string]interface{}{"enabled": false},
		},
	); err != nil {
		return err
	}

	// --- Tempo ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing Tempo (%s-tempo)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-tempo", prefix),
		"tempo",
		"1.24.4",
		"https://grafana.github.io/helm-charts",
		map[string]interface{}{
			"tempo": map[string]interface{}{
				"storage": map[string]interface{}{
					"trace": map[string]interface{}{
						"backend": "local",
					},
				},
			},
			"persistence": map[string]interface{}{
				"enabled": true,
				"size":    "2Gi",
			},
		},
	); err != nil {
		return err
	}

	// --- Alloy ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing Alloy (%s-alloy)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-alloy", prefix),
		"alloy",
		"1.6.0",
		"https://grafana.github.io/helm-charts",
		map[string]interface{}{
			"alloy": map[string]interface{}{
				"configMap": map[string]interface{}{
					"content": fmt.Sprintf(`otelcol.receiver.otlp "default" {
  grpc { endpoint = "0.0.0.0:4317" }
  http { endpoint = "0.0.0.0:4318" }
  output { traces = [otelcol.exporter.otlp.tempo.input] }
}
otelcol.exporter.otlp "tempo" {
  client { endpoint = "%s-tempo:4317" }
}
`, prefix),
				},
			},
			"controller": map[string]interface{}{
				"type": "deployment",
			},
			"service": map[string]interface{}{
				"enabled": true,
			},
		},
	); err != nil {
		return err
	}

	// --- Quickwit (non-critical: heavy multi-pod chart, may time out on low-resource clusters) ---
	// Use a short timeout so failure doesn't block the rest of setup for 15 minutes.
	quickwitCtx, quickwitCancel := context.WithTimeout(ctx, 2*time.Minute)
	defer quickwitCancel()
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing Quickwit (%s-quickwit)...", prefix)})
	if err := helm.InstallOrUpgrade(quickwitCtx,
		fmt.Sprintf("%s-quickwit", prefix),
		"quickwit",
		"0.7.21",
		"https://helm.quickwit.io",
		map[string]interface{}{
			"config": map[string]interface{}{
				"default_index_root_uri": fmt.Sprintf("s3://quickwit/indexes?endpoint=http://%s-minio:9000&force_path_style_access=true", prefix),
				"storage": map[string]interface{}{
					"s3": map[string]interface{}{
						"endpoint":              fmt.Sprintf("http://%s-minio:9000", prefix),
						"access_key_id":         cfg.MinioUser,
						"secret_access_key":     cfg.MinioPassword,
						"force_path_style_access": true,
					},
				},
			},
			"searcher":      map[string]interface{}{"replicaCount": 1},
			"indexer":       map[string]interface{}{"replicaCount": 1},
			"control_plane": map[string]interface{}{"replicaCount": 1},
			"janitor":       map[string]interface{}{"replicaCount": 1},
			"metastore":     map[string]interface{}{"replicaCount": 1},
		},
	); err != nil {
		send(ProgressEvent{Kind: EventWarn, Name: fmt.Sprintf("Quickwit install failed (non-critical): %v", err)})
	}

	// --- Keel ---
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Installing Keel (%s-keel)...", prefix)})
	if err := helm.InstallOrUpgrade(ctx,
		fmt.Sprintf("%s-keel", prefix),
		"keel",
		"",
		"https://charts.keel.sh",
		map[string]interface{}{
			"service": map[string]interface{}{"enabled": true},
		},
	); err != nil {
		// Keel is non-critical, log and continue
		send(ProgressEvent{Kind: EventWarn, Name: fmt.Sprintf("Keel install failed (non-critical): %v", err)})
	}

	_ = ns // used implicitly via prefix
	return nil
}

// DeployCustomServices deploys custom K8s resources (non-Helm services).
func DeployCustomServices(ctx context.Context, cfg Config, kubeconfigPath string, send Sender) error {
	rc, err := clientcmd.BuildConfigFromFlags("", kubeconfigPath)
	if err != nil {
		return fmt.Errorf("build rest config: %w", err)
	}
	kc, err := kubernetes.NewForConfig(rc)
	if err != nil {
		return fmt.Errorf("create kubernetes client: %w", err)
	}

	prefix := cfg.Prefix()
	ns := cfg.Namespace

	// Ensure namespace exists
	if err := ensureNamespace(ctx, kc, ns); err != nil {
		return err
	}

	// ChromaDB
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Deploying ChromaDB (%s-chromadb)...", prefix)})
	if err := deployChromaDB(ctx, kc, prefix, ns); err != nil {
		return err
	}

	// Crawl4AI
	send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Deploying Crawl4AI (%s-crawl4ai)...", prefix)})
	if err := deployCrawl4AI(ctx, kc, prefix, ns); err != nil {
		return err
	}

	// Memos (with its own postgres)
	if cfg.EnableMemos {
		send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Deploying Memos (%s-memos)...", prefix)})
		if err := deployMemos(ctx, kc, prefix, ns); err != nil {
			return err
		}
	}

	// Hindsight (with pgvector postgres)
	if cfg.EnableHindsight {
		send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Deploying Hindsight (%s-hindsight)...", prefix)})
		if err := deployHindsight(ctx, kc, prefix, ns, cfg); err != nil {
			return err
		}
	}

	// Mem0
	if cfg.EnableMem0 {
		send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Deploying Mem0 (%s-mem0)...", prefix)})
		if err := deployMem0(ctx, kc, prefix, ns, cfg); err != nil {
			return err
		}
	}

	// Ollama
	if cfg.EnableOllama {
		send(ProgressEvent{Kind: EventInfo, Name: fmt.Sprintf("Deploying Ollama (%s-ollama)...", prefix)})
		if err := deployOllama(ctx, kc, prefix, ns); err != nil {
			return err
		}
	}

	return nil
}

// ensureNamespace creates the namespace if it doesn't exist.
func ensureNamespace(ctx context.Context, kc *kubernetes.Clientset, ns string) error {
	_, err := kc.CoreV1().Namespaces().Get(ctx, ns, metav1.GetOptions{})
	if err == nil {
		return nil
	}
	if !k8serrors.IsNotFound(err) {
		return fmt.Errorf("get namespace: %w", err)
	}
	_, err = kc.CoreV1().Namespaces().Create(ctx, &corev1.Namespace{
		ObjectMeta: metav1.ObjectMeta{Name: ns},
	}, metav1.CreateOptions{})
	if err != nil && !k8serrors.IsAlreadyExists(err) {
		return fmt.Errorf("create namespace %s: %w", ns, err)
	}
	return nil
}

// applyOrUpdateDeployment creates or updates a Deployment.
func applyOrUpdateDeployment(ctx context.Context, kc *kubernetes.Clientset, ns string, dep *appsv1.Deployment) error {
	_, err := kc.AppsV1().Deployments(ns).Get(ctx, dep.Name, metav1.GetOptions{})
	if k8serrors.IsNotFound(err) {
		_, err = kc.AppsV1().Deployments(ns).Create(ctx, dep, metav1.CreateOptions{})
		return err
	}
	if err != nil {
		return err
	}
	_, err = kc.AppsV1().Deployments(ns).Update(ctx, dep, metav1.UpdateOptions{})
	return err
}

// applyOrUpdateService creates or updates a Service.
func applyOrUpdateService(ctx context.Context, kc *kubernetes.Clientset, ns string, svc *corev1.Service) error {
	existing, err := kc.CoreV1().Services(ns).Get(ctx, svc.Name, metav1.GetOptions{})
	if k8serrors.IsNotFound(err) {
		_, err = kc.CoreV1().Services(ns).Create(ctx, svc, metav1.CreateOptions{})
		return err
	}
	if err != nil {
		return err
	}
	svc.ResourceVersion = existing.ResourceVersion
	svc.Spec.ClusterIP = existing.Spec.ClusterIP
	_, err = kc.CoreV1().Services(ns).Update(ctx, svc, metav1.UpdateOptions{})
	return err
}

// applyOrUpdatePVC creates a PVC if it doesn't exist (PVCs are not updated).
func applyOrUpdatePVC(ctx context.Context, kc *kubernetes.Clientset, ns string, pvc *corev1.PersistentVolumeClaim) error {
	_, err := kc.CoreV1().PersistentVolumeClaims(ns).Get(ctx, pvc.Name, metav1.GetOptions{})
	if k8serrors.IsNotFound(err) {
		_, err = kc.CoreV1().PersistentVolumeClaims(ns).Create(ctx, pvc, metav1.CreateOptions{})
		return err
	}
	return err // Already exists is fine
}

// applyOrUpdateConfigMap creates or updates a ConfigMap.
func applyOrUpdateConfigMap(ctx context.Context, kc *kubernetes.Clientset, ns string, cm *corev1.ConfigMap) error {
	_, err := kc.CoreV1().ConfigMaps(ns).Get(ctx, cm.Name, metav1.GetOptions{})
	if k8serrors.IsNotFound(err) {
		_, err = kc.CoreV1().ConfigMaps(ns).Create(ctx, cm, metav1.CreateOptions{})
		return err
	}
	if err != nil {
		return err
	}
	_, err = kc.CoreV1().ConfigMaps(ns).Update(ctx, cm, metav1.UpdateOptions{})
	return err
}

// resourceList builds a ResourceList from cpu and memory strings (empty strings are omitted).
func resourceList(cpu, mem string) corev1.ResourceList {
	rl := corev1.ResourceList{}
	if cpu != "" {
		rl[corev1.ResourceCPU] = resource.MustParse(cpu)
	}
	if mem != "" {
		rl[corev1.ResourceMemory] = resource.MustParse(mem)
	}
	return rl
}

// storageList builds a ResourceList for PVC storage requests.
func storageList(size string) corev1.ResourceList {
	return corev1.ResourceList{
		corev1.ResourceStorage: resource.MustParse(size),
	}
}

// ptr returns a pointer to v.
func ptr[T any](v T) *T { return &v }

// stdLabels returns standard k8s labels.
func stdLabels(component, instance string) map[string]string {
	return map[string]string{
		"app.kubernetes.io/name":      component,
		"app.kubernetes.io/instance":  instance,
		"app.kubernetes.io/component": component,
	}
}

// deployChromaDB deploys ChromaDB Deployment, Service, and PVC.
func deployChromaDB(ctx context.Context, kc *kubernetes.Clientset, prefix, ns string) error {
	name := fmt.Sprintf("%s-chromadb", prefix)
	labels := stdLabels("chromadb", prefix)

	if err := applyOrUpdatePVC(ctx, kc, ns, &corev1.PersistentVolumeClaim{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: corev1.PersistentVolumeClaimSpec{
			AccessModes: []corev1.PersistentVolumeAccessMode{corev1.ReadWriteOnce},
			Resources:   corev1.VolumeResourceRequirements{Requests: storageList("1Gi")},
		},
	}); err != nil {
		return err
	}

	if err := applyOrUpdateDeployment(ctx, kc, ns, &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr(int32(1)),
			Strategy: appsv1.DeploymentStrategy{Type: appsv1.RecreateDeploymentStrategyType},
			Selector: &metav1.LabelSelector{MatchLabels: labels},
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{Labels: labels},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name:            "chromadb",
						Image:           "chromadb/chroma:latest",
						ImagePullPolicy: corev1.PullIfNotPresent,
						Ports:           []corev1.ContainerPort{{Name: "http", ContainerPort: 8000, Protocol: corev1.ProtocolTCP}},
						Env: []corev1.EnvVar{
							{Name: "IS_PERSISTENT", Value: "TRUE"},
							{Name: "PERSIST_DIRECTORY", Value: "/chroma/chroma"},
							{Name: "ANONYMIZED_TELEMETRY", Value: "FALSE"},
						},
						VolumeMounts: []corev1.VolumeMount{{Name: "data", MountPath: "/chroma/chroma"}},
						Resources: corev1.ResourceRequirements{
							Requests: resourceList("100m", "256Mi"),
							Limits:   resourceList("500m", "1Gi"),
						},
					}},
					Volumes: []corev1.Volume{{
						Name: "data",
						VolumeSource: corev1.VolumeSource{
							PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSource{ClaimName: name},
						},
					}},
				},
			},
		},
	}); err != nil {
		return err
	}

	return applyOrUpdateService(ctx, kc, ns, &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: corev1.ServiceSpec{
			Type:     corev1.ServiceTypeClusterIP,
			Selector: labels,
			Ports:    []corev1.ServicePort{{Name: "http", Port: 8000, TargetPort: intstr.FromString("http"), Protocol: corev1.ProtocolTCP}},
		},
	})
}

// deployCrawl4AI deploys Crawl4AI Deployment and Service.
func deployCrawl4AI(ctx context.Context, kc *kubernetes.Clientset, prefix, ns string) error {
	name := fmt.Sprintf("%s-crawl4ai", prefix)
	labels := stdLabels("crawl4ai", prefix)

	if err := applyOrUpdateDeployment(ctx, kc, ns, &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr(int32(1)),
			Selector: &metav1.LabelSelector{MatchLabels: labels},
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{Labels: labels},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name:            "crawl4ai",
						Image:           "unclecode/crawl4ai:latest",
						ImagePullPolicy: corev1.PullIfNotPresent,
						Ports:           []corev1.ContainerPort{{Name: "http", ContainerPort: 11235, Protocol: corev1.ProtocolTCP}},
						Resources: corev1.ResourceRequirements{
							Requests: resourceList("100m", "256Mi"),
							Limits:   resourceList("1", "1Gi"),
						},
					}},
				},
			},
		},
	}); err != nil {
		return err
	}

	return applyOrUpdateService(ctx, kc, ns, &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: corev1.ServiceSpec{
			Type:     corev1.ServiceTypeClusterIP,
			Selector: labels,
			Ports:    []corev1.ServicePort{{Name: "http", Port: 11235, TargetPort: intstr.FromString("http"), Protocol: corev1.ProtocolTCP}},
		},
	})
}

// deployMemos deploys Memos + its own Postgres.
func deployMemos(ctx context.Context, kc *kubernetes.Clientset, prefix, ns string) error {
	pgName := fmt.Sprintf("%s-memos-pg", prefix)
	memosName := fmt.Sprintf("%s-memos", prefix)
	pgLabels := stdLabels("memos-pg", prefix)
	memosLabels := stdLabels("memos", prefix)

	// Postgres PVC
	if err := applyOrUpdatePVC(ctx, kc, ns, &corev1.PersistentVolumeClaim{
		ObjectMeta: metav1.ObjectMeta{Name: pgName, Namespace: ns, Labels: pgLabels},
		Spec: corev1.PersistentVolumeClaimSpec{
			AccessModes: []corev1.PersistentVolumeAccessMode{corev1.ReadWriteOnce},
			Resources:   corev1.VolumeResourceRequirements{Requests: storageList("2Gi")},
		},
	}); err != nil {
		return err
	}

	// Postgres Deployment
	if err := applyOrUpdateDeployment(ctx, kc, ns, &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{Name: pgName, Namespace: ns, Labels: pgLabels},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr(int32(1)),
			Strategy: appsv1.DeploymentStrategy{Type: appsv1.RecreateDeploymentStrategyType},
			Selector: &metav1.LabelSelector{MatchLabels: pgLabels},
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{Labels: pgLabels},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name:            "postgres",
						Image:           "postgres:16-alpine",
						ImagePullPolicy: corev1.PullIfNotPresent,
						Ports:           []corev1.ContainerPort{{Name: "postgres", ContainerPort: 5432, Protocol: corev1.ProtocolTCP}},
						Env: []corev1.EnvVar{
							{Name: "POSTGRES_USER", Value: "memos"},
							{Name: "POSTGRES_PASSWORD", Value: "memos"},
							{Name: "POSTGRES_DB", Value: "memos"},
							{Name: "PGDATA", Value: "/var/lib/postgresql/data/pgdata"},
						},
						VolumeMounts: []corev1.VolumeMount{{Name: "data", MountPath: "/var/lib/postgresql/data"}},
						Resources: corev1.ResourceRequirements{
							Requests: resourceList("50m", "64Mi"),
							Limits:   resourceList("250m", "256Mi"),
						},
					}},
					Volumes: []corev1.Volume{{
						Name: "data",
						VolumeSource: corev1.VolumeSource{
							PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSource{ClaimName: pgName},
						},
					}},
				},
			},
		},
	}); err != nil {
		return err
	}

	// Postgres Service
	if err := applyOrUpdateService(ctx, kc, ns, &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{Name: pgName, Namespace: ns, Labels: pgLabels},
		Spec: corev1.ServiceSpec{
			Type:     corev1.ServiceTypeClusterIP,
			Selector: pgLabels,
			Ports:    []corev1.ServicePort{{Name: "postgres", Port: 5432, TargetPort: intstr.FromString("postgres"), Protocol: corev1.ProtocolTCP}},
		},
	}); err != nil {
		return err
	}

	// Memos Deployment
	if err := applyOrUpdateDeployment(ctx, kc, ns, &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{Name: memosName, Namespace: ns, Labels: memosLabels},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr(int32(1)),
			Selector: &metav1.LabelSelector{MatchLabels: memosLabels},
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{Labels: memosLabels},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name:            "memos",
						Image:           "neosmemo/memos:stable",
						ImagePullPolicy: corev1.PullIfNotPresent,
						Ports:           []corev1.ContainerPort{{Name: "http", ContainerPort: 5230, Protocol: corev1.ProtocolTCP}},
						Env: []corev1.EnvVar{
							{Name: "MEMOS_DRIVER", Value: "postgres"},
							{Name: "MEMOS_DSN", Value: fmt.Sprintf("postgresql://memos:memos@%s:5432/memos?sslmode=disable", pgName)},
						},
						Resources: corev1.ResourceRequirements{
							Requests: resourceList("50m", "64Mi"),
							Limits:   resourceList("250m", "256Mi"),
						},
					}},
				},
			},
		},
	}); err != nil {
		return err
	}

	// Memos Service
	return applyOrUpdateService(ctx, kc, ns, &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{Name: memosName, Namespace: ns, Labels: memosLabels},
		Spec: corev1.ServiceSpec{
			Type:     corev1.ServiceTypeClusterIP,
			Selector: memosLabels,
			Ports:    []corev1.ServicePort{{Name: "http", Port: 5230, TargetPort: intstr.FromString("http"), Protocol: corev1.ProtocolTCP}},
		},
	})
}

// deployHindsight deploys Hindsight + pgvector Postgres.
func deployHindsight(ctx context.Context, kc *kubernetes.Clientset, prefix, ns string, cfg Config) error {
	pgName := fmt.Sprintf("%s-hindsight-pg", prefix)
	hindsightName := fmt.Sprintf("%s-hindsight", prefix)
	pgLabels := stdLabels("hindsight-pg", prefix)
	hindsightLabels := stdLabels("hindsight", prefix)

	// PVC
	if err := applyOrUpdatePVC(ctx, kc, ns, &corev1.PersistentVolumeClaim{
		ObjectMeta: metav1.ObjectMeta{Name: pgName, Namespace: ns, Labels: pgLabels},
		Spec: corev1.PersistentVolumeClaimSpec{
			AccessModes: []corev1.PersistentVolumeAccessMode{corev1.ReadWriteOnce},
			Resources:   corev1.VolumeResourceRequirements{Requests: storageList("5Gi")},
		},
	}); err != nil {
		return err
	}

	// pgvector Deployment
	if err := applyOrUpdateDeployment(ctx, kc, ns, &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{Name: pgName, Namespace: ns, Labels: pgLabels},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr(int32(1)),
			Strategy: appsv1.DeploymentStrategy{Type: appsv1.RecreateDeploymentStrategyType},
			Selector: &metav1.LabelSelector{MatchLabels: pgLabels},
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{Labels: pgLabels},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name:            "postgres",
						Image:           "pgvector/pgvector:pg16",
						ImagePullPolicy: corev1.PullIfNotPresent,
						Ports:           []corev1.ContainerPort{{Name: "postgres", ContainerPort: 5432, Protocol: corev1.ProtocolTCP}},
						Env: []corev1.EnvVar{
							{Name: "POSTGRES_USER", Value: "hindsight"},
							{Name: "POSTGRES_PASSWORD", Value: "hindsight"},
							{Name: "POSTGRES_DB", Value: "hindsight"},
							{Name: "PGDATA", Value: "/var/lib/postgresql/data/pgdata"},
						},
						VolumeMounts: []corev1.VolumeMount{{Name: "data", MountPath: "/var/lib/postgresql/data"}},
						Resources: corev1.ResourceRequirements{
							Requests: resourceList("50m", "64Mi"),
							Limits:   resourceList("250m", "256Mi"),
						},
					}},
					Volumes: []corev1.Volume{{
						Name: "data",
						VolumeSource: corev1.VolumeSource{
							PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSource{ClaimName: pgName},
						},
					}},
				},
			},
		},
	}); err != nil {
		return err
	}

	// pgvector Service
	if err := applyOrUpdateService(ctx, kc, ns, &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{Name: pgName, Namespace: ns, Labels: pgLabels},
		Spec: corev1.ServiceSpec{
			Type:     corev1.ServiceTypeClusterIP,
			Selector: pgLabels,
			Ports:    []corev1.ServicePort{{Name: "postgres", Port: 5432, TargetPort: intstr.FromString("postgres"), Protocol: corev1.ProtocolTCP}},
		},
	}); err != nil {
		return err
	}

	// Hindsight Deployment
	if err := applyOrUpdateDeployment(ctx, kc, ns, &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{Name: hindsightName, Namespace: ns, Labels: hindsightLabels},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr(int32(1)),
			Selector: &metav1.LabelSelector{MatchLabels: hindsightLabels},
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{Labels: hindsightLabels},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name:            "hindsight",
						Image:           "ghcr.io/vectorize-io/hindsight:latest",
						ImagePullPolicy: corev1.PullIfNotPresent,
						Ports: []corev1.ContainerPort{
							{Name: "http", ContainerPort: 8888, Protocol: corev1.ProtocolTCP},
							{Name: "admin", ContainerPort: 9999, Protocol: corev1.ProtocolTCP},
						},
						Env: []corev1.EnvVar{
							{Name: "HINDSIGHT_DB_URL", Value: fmt.Sprintf("postgresql://hindsight:hindsight@%s:5432/hindsight", pgName)},
							{Name: "HINDSIGHT_API_LLM_PROVIDER", Value: cfg.HindsightLLMProvider},
							{Name: "HINDSIGHT_API_LLM_MODEL", Value: cfg.HindsightLLMModel},
							{Name: "HINDSIGHT_API_LLM_BASE_URL", Value: cfg.HindsightLLMBaseURL},
							{Name: "HINDSIGHT_API_LLM_API_KEY", Value: ""},
						},
						Resources: corev1.ResourceRequirements{
							Requests: resourceList("100m", "256Mi"),
							Limits:   resourceList("500m", "1Gi"),
						},
					}},
				},
			},
		},
	}); err != nil {
		return err
	}

	return applyOrUpdateService(ctx, kc, ns, &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{Name: hindsightName, Namespace: ns, Labels: hindsightLabels},
		Spec: corev1.ServiceSpec{
			Type:     corev1.ServiceTypeClusterIP,
			Selector: hindsightLabels,
			Ports:    []corev1.ServicePort{{Name: "http", Port: 8888, TargetPort: intstr.FromString("http"), Protocol: corev1.ProtocolTCP}},
		},
	})
}

// deployMem0 deploys Mem0 with its ConfigMap.
func deployMem0(ctx context.Context, kc *kubernetes.Clientset, prefix, ns string, cfg Config) error {
	name := fmt.Sprintf("%s-mem0", prefix)
	configMapName := fmt.Sprintf("%s-mem0-config", prefix)
	chromaDBName := fmt.Sprintf("%s-chromadb", prefix)
	labels := stdLabels("mem0", prefix)

	patchScript := buildMem0PatchScript()

	if err := applyOrUpdateConfigMap(ctx, kc, ns, &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{Name: configMapName, Namespace: ns, Labels: labels},
		Data:       map[string]string{"patch_config.py": patchScript},
	}); err != nil {
		return err
	}

	if err := applyOrUpdateDeployment(ctx, kc, ns, &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr(int32(1)),
			Selector: &metav1.LabelSelector{MatchLabels: labels},
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{Labels: labels},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name:            "mem0",
						Image:           "mem0/mem0-api-server:latest",
						ImagePullPolicy: corev1.PullIfNotPresent,
						Command:         []string{"sh", "-c"},
						Args: []string{
							"pip install --quiet --no-cache-dir chromadb ollama && python3 /app/patch_config.py && exec uvicorn main:app --host 0.0.0.0 --port 8000 --workers 1",
						},
						WorkingDir: "/app",
						Ports:      []corev1.ContainerPort{{Name: "http", ContainerPort: 8000, Protocol: corev1.ProtocolTCP}},
						Env: []corev1.EnvVar{
							{Name: "CHROMA_HOST", Value: chromaDBName},
							{Name: "CHROMA_PORT", Value: "8000"},
							{Name: "HISTORY_DB_PATH", Value: "/tmp/mem0_history.db"},
							{Name: "OLLAMA_BASE_URL", Value: cfg.Mem0OllamaBaseURL},
							{Name: "MEM0_OLLAMA_BASE_URL", Value: cfg.Mem0OllamaBaseURL},
							{Name: "MEM0_OLLAMA_LLM_MODEL", Value: cfg.Mem0OllamaModel},
						},
						VolumeMounts: []corev1.VolumeMount{{
							Name:      "patch-script",
							MountPath: "/app/patch_config.py",
							SubPath:   "patch_config.py",
							ReadOnly:  true,
						}},
						Resources: corev1.ResourceRequirements{
							Requests: resourceList("250m", "256Mi"),
							Limits:   resourceList("1", "1Gi"),
						},
					}},
					Volumes: []corev1.Volume{{
						Name: "patch-script",
						VolumeSource: corev1.VolumeSource{
							ConfigMap: &corev1.ConfigMapVolumeSource{
								LocalObjectReference: corev1.LocalObjectReference{Name: configMapName},
							},
						},
					}},
				},
			},
		},
	}); err != nil {
		return err
	}

	return applyOrUpdateService(ctx, kc, ns, &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: corev1.ServiceSpec{
			Type:     corev1.ServiceTypeClusterIP,
			Selector: labels,
			Ports:    []corev1.ServicePort{{Name: "http", Port: 8000, TargetPort: intstr.FromString("http"), Protocol: corev1.ProtocolTCP}},
		},
	})
}

// deployOllama deploys Ollama with persistent storage.
func deployOllama(ctx context.Context, kc *kubernetes.Clientset, prefix, ns string) error {
	name := fmt.Sprintf("%s-ollama", prefix)
	labels := stdLabels("ollama", prefix)

	if err := applyOrUpdatePVC(ctx, kc, ns, &corev1.PersistentVolumeClaim{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: corev1.PersistentVolumeClaimSpec{
			AccessModes: []corev1.PersistentVolumeAccessMode{corev1.ReadWriteOnce},
			Resources:   corev1.VolumeResourceRequirements{Requests: storageList("20Gi")},
		},
	}); err != nil {
		return err
	}

	if err := applyOrUpdateDeployment(ctx, kc, ns, &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr(int32(1)),
			Strategy: appsv1.DeploymentStrategy{Type: appsv1.RecreateDeploymentStrategyType},
			Selector: &metav1.LabelSelector{MatchLabels: labels},
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{Labels: labels},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name:            "ollama",
						Image:           "ollama/ollama:latest",
						ImagePullPolicy: corev1.PullIfNotPresent,
						Command:         []string{"ollama", "serve"},
						Ports:           []corev1.ContainerPort{{Name: "http", ContainerPort: 11434, Protocol: corev1.ProtocolTCP}},
						Env: []corev1.EnvVar{
							{Name: "OLLAMA_HOST", Value: "0.0.0.0:11434"},
						},
						VolumeMounts: []corev1.VolumeMount{{Name: "ollama-data", MountPath: "/root/.ollama"}},
						Resources: corev1.ResourceRequirements{
							Requests: resourceList("100m", "256Mi"),
							Limits:   resourceList("2", "4Gi"),
						},
					}},
					Volumes: []corev1.Volume{{
						Name: "ollama-data",
						VolumeSource: corev1.VolumeSource{
							PersistentVolumeClaim: &corev1.PersistentVolumeClaimVolumeSource{ClaimName: name},
						},
					}},
				},
			},
		},
	}); err != nil {
		return err
	}

	return applyOrUpdateService(ctx, kc, ns, &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: ns, Labels: labels},
		Spec: corev1.ServiceSpec{
			Type:     corev1.ServiceTypeClusterIP,
			Selector: labels,
			Ports:    []corev1.ServicePort{{Name: "http", Port: 11434, TargetPort: intstr.FromString("http"), Protocol: corev1.ProtocolTCP}},
		},
	})
}

// WaitForDeployment waits until a deployment has the desired number of available replicas.
func WaitForDeployment(ctx context.Context, rc *rest.Config, ns, name string) error {
	kc, err := kubernetes.NewForConfig(rc)
	if err != nil {
		return err
	}
	return wait.PollUntilContextTimeout(ctx, 5*time.Second, 10*time.Minute, true, func(ctx context.Context) (bool, error) {
		dep, err := kc.AppsV1().Deployments(ns).Get(ctx, name, metav1.GetOptions{})
		if err != nil {
			return false, nil
		}
		if dep.Spec.Replicas == nil {
			return false, nil
		}
		return dep.Status.AvailableReplicas >= *dep.Spec.Replicas, nil
	})
}

// buildMem0PatchScript returns the patch_config.py script for Mem0.
func buildMem0PatchScript() string {
	return strings.TrimSpace(`
import json
import os
import pathlib
import re

MAIN_PATH = pathlib.Path("/app/main.py")

def _env(name, default):
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
        "config": {"host": chroma_host, "port": chroma_port, "collection_name": "memories"},
    },
    "llm": {
        "provider": "ollama",
        "config": {"model": llm_model, "ollama_base_url": ollama_base_url, "temperature": 0.0, "max_tokens": 256},
    },
    "embedder": {
        "provider": "ollama",
        "config": {"model": embed_model, "ollama_base_url": ollama_base_url},
    },
    "history_db_path": history_db_path,
}

replacement = (
    "DEFAULT_CONFIG = " + json.dumps(config, indent=4) +
    "\n\n\ntry:\n"
    "    MEMORY_INSTANCE = Memory.from_config(DEFAULT_CONFIG)\n"
    "except Exception as e:\n"
    "    logging.exception('Mem0 startup initialization failed')\n"
    "    MEMORY_INSTANCE = None\n"
)

content = MAIN_PATH.read_text()
updated, count = re.subn(
    r"DEFAULT_CONFIG = \{.*?\nMEMORY_INSTANCE = Memory\.from_config\(DEFAULT_CONFIG\)",
    replacement, content, count=1, flags=re.S,
)
if count != 1:
    raise RuntimeError("Failed to patch mem0 main.py DEFAULT_CONFIG block")

MAIN_PATH.write_text(updated)
print(f"Patched /app/main.py: CHROMA={chroma_host}:{chroma_port} OLLAMA={ollama_base_url}")
`) + "\n"
}
