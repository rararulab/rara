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

package doctor

// Config holds the doctor command configuration from CLI flags.
type Config struct {
	Namespace string
	Release   string
	Domain    string // empty = derive from Helm values, fallback "rara.local"
}

// HelmValues represents the relevant subset of Helm values for doctor checks.
type HelmValues struct {
	Mem0Enabled      bool
	MemosEnabled     bool
	HindsightEnabled bool
	OllamaEnabled    bool
	Domain           string
}

// DefaultHelmValues returns fallback values matching the chart defaults.
func DefaultHelmValues() HelmValues {
	return HelmValues{
		Mem0Enabled:      false,
		MemosEnabled:     true,
		HindsightEnabled: true,
		OllamaEnabled:    false,
		Domain:           "rara.local",
	}
}

// PodCheckDef defines a pod readiness check.
type PodCheckDef struct {
	Name          string
	LabelSelector string
	EnabledFunc   func(HelmValues) bool // nil = always enabled
}

// EndpointCheckDef defines a kube-apiserver proxy endpoint check.
type EndpointCheckDef struct {
	Name        string
	ServiceName string // fully qualified service name
	Port        string
	Path        string
}

// IngressCheckDef defines an HTTPS ingress endpoint check.
type IngressCheckDef struct {
	Subdomain   string
	Path        string
	EnabledFunc func(HelmValues) bool // nil = always enabled
}

// --- Pod check definitions ---

var coreInfraPods = []PodCheckDef{
	{Name: "PostgreSQL", LabelSelector: "app.kubernetes.io/name=postgresql"},
	{Name: "MinIO", LabelSelector: "app=minio"},
	{Name: "ChromaDB", LabelSelector: "app.kubernetes.io/name=chromadb"},
	{Name: "Crawl4AI", LabelSelector: "app.kubernetes.io/name=crawl4ai"},
}

var observabilityPods = []PodCheckDef{
	{Name: "Prometheus", LabelSelector: "app.kubernetes.io/name=prometheus"},
	{Name: "Grafana", LabelSelector: "app.kubernetes.io/name=grafana"},
	{Name: "AlertManager", LabelSelector: "app.kubernetes.io/name=alertmanager"},
	{Name: "Tempo", LabelSelector: "app.kubernetes.io/name=tempo"},
	{Name: "Alloy", LabelSelector: "app.kubernetes.io/name=alloy"},
	{Name: "Quickwit", LabelSelector: "app.kubernetes.io/name=quickwit"},
}

var platformPods = []PodCheckDef{
	{Name: "Langfuse Web", LabelSelector: "app=web,app.kubernetes.io/name=langfuse"},
	{Name: "Langfuse Worker", LabelSelector: "app=worker,app.kubernetes.io/name=langfuse"},
	{Name: "Langfuse PG", LabelSelector: "statefulset.kubernetes.io/pod-name=rara-infra-langfuse-pg-0"},
	{Name: "Redis", LabelSelector: "app.kubernetes.io/name=redis"},
	{Name: "Consul Server", LabelSelector: "app=consul,component=server"},
	{Name: "Consul Client", LabelSelector: "app=consul,component=client"},
}

// --- HTTPS ingress endpoint definitions ---

var ingressEndpoints = []IngressCheckDef{
	{Subdomain: "grafana", Path: "/"},
	{Subdomain: "prometheus", Path: "/"},
	{Subdomain: "alertmanager", Path: "/"},
	{Subdomain: "langfuse", Path: "/"},
	{Subdomain: "quickwit", Path: "/"},
	{Subdomain: "consul", Path: "/"},
	{Subdomain: "minio", Path: "/"},
	{Subdomain: "traefik", Path: "/"},
	{Subdomain: "memos", Path: "/", EnabledFunc: func(v HelmValues) bool { return v.MemosEnabled }},
	{Subdomain: "hindsight", Path: "/metrics", EnabledFunc: func(v HelmValues) bool { return v.HindsightEnabled }},
	{Subdomain: "ollama", Path: "/api/tags", EnabledFunc: func(v HelmValues) bool { return v.OllamaEnabled }},
}
