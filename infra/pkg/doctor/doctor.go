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

import (
	"context"
	"fmt"

	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
)

// Status represents the outcome of a single check.
type Status int

const (
	StatusPass Status = iota
	StatusFail
	StatusSkip
	StatusWarn
)

// CheckResult holds the outcome of one check.
type CheckResult struct {
	Name   string
	Status Status
	Detail string
}

// SectionResult groups checks under a titled section.
type SectionResult struct {
	Title  string
	Icon   string
	Checks []CheckResult
}

// Report collects all section results for a doctor run.
type Report struct {
	Namespace string
	Domain    string
	LBIP      string
	Sections  []SectionResult
}

// Summary returns counts of passed, failed, and skipped checks.
func (r *Report) Summary() (pass, fail, skip int) {
	for _, s := range r.Sections {
		for _, c := range s.Checks {
			switch c.Status {
			case StatusPass, StatusWarn:
				pass++
			case StatusFail:
				fail++
			case StatusSkip:
				skip++
			}
		}
	}
	return
}

// HasFailures returns true if any check failed.
func (r *Report) HasFailures() bool {
	_, fail, _ := r.Summary()
	return fail > 0
}

// Run executes all doctor checks and returns a Report.
func Run(ctx context.Context, cs *kubernetes.Clientset, rc *rest.Config, cfg Config) (*Report, error) {
	c := &checker{cs: cs, restConfig: rc, cfg: cfg}

	// Read Helm values to determine which services are enabled.
	values, err := ReadHelmValues(ctx, cs, cfg.Namespace, cfg.Release)
	if err != nil {
		// Warn but continue with defaults.
		values = DefaultHelmValues()
	}
	c.values = values

	// If domain not set via CLI flag, use Helm values.
	if cfg.Domain == "" {
		cfg.Domain = values.Domain
		if cfg.Domain == "" {
			cfg.Domain = "rara.local"
		}
		c.cfg = cfg
	}

	report := &Report{Namespace: cfg.Namespace, Domain: cfg.Domain}

	// 1. Helm Release
	report.addSection(c.runHelmSection(ctx))

	// 2. Core Infrastructure
	report.addSection(c.runPodSection(ctx, "Core Infrastructure", "🗄️", coreInfraPods))

	// 3. Memory Services (pods + endpoints, interleaved)
	report.addSection(c.runMemorySection(ctx))

	// 4. Ingress & TLS
	report.addSection(c.runIngressTLSSection(ctx))

	// 5. Observability
	report.addSection(c.runPodSection(ctx, "Observability", "📈", observabilityPods))

	// 6. Platform Services
	report.addSection(c.runPlatformSection(ctx))

	// 7. Persistent Volumes
	report.addSection(SectionResult{
		Title:  "Persistent Volumes",
		Icon:   "💾",
		Checks: []CheckResult{c.checkPVCs(ctx)},
	})

	// 8. HTTPS Endpoints
	lbIP, err := c.getTraefikLBIP(ctx)
	if err != nil {
		lbIP = "pending"
	}
	report.LBIP = lbIP
	report.addSection(c.runHTTPSEndpointSection(ctx, lbIP))

	return report, nil
}

func (r *Report) addSection(s SectionResult) {
	r.Sections = append(r.Sections, s)
}

// runHelmSection checks the Helm release status.
func (c *checker) runHelmSection(ctx context.Context) SectionResult {
	return SectionResult{
		Title:  "Helm Release",
		Icon:   "📦",
		Checks: []CheckResult{c.checkHelmRelease(ctx)},
	}
}

// runPodSection runs pod readiness checks for a section, respecting enabled flags.
func (c *checker) runPodSection(ctx context.Context, title, icon string, defs []PodCheckDef) SectionResult {
	var checks []CheckResult
	for _, def := range defs {
		if def.EnabledFunc != nil && !def.EnabledFunc(c.values) {
			checks = append(checks, CheckResult{
				Name: def.Name, Status: StatusSkip, Detail: "disabled in Helm values",
			})
			continue
		}
		checks = append(checks, c.checkPodReady(ctx, def))
	}
	return SectionResult{Title: title, Icon: icon, Checks: checks}
}

// runMemorySection checks memory services with interleaved endpoint checks.
func (c *checker) runMemorySection(ctx context.Context) SectionResult {
	var checks []CheckResult

	// Mem0
	if c.values.Mem0Enabled {
		checks = append(checks, c.checkPodReady(ctx, PodCheckDef{
			Name: "Mem0", LabelSelector: "app.kubernetes.io/component=mem0",
		}))
		checks = append(checks, c.checkServiceEndpoint(ctx, EndpointCheckDef{
			Name: "Mem0 API", ServiceName: c.cfg.Release + "-mem0", Port: "8000", Path: "/",
		}))
	} else {
		checks = append(checks, CheckResult{Name: "Mem0", Status: StatusSkip, Detail: "disabled in Helm values"})
	}

	// Memos
	if c.values.MemosEnabled {
		checks = append(checks, c.checkPodReady(ctx, PodCheckDef{
			Name: "Memos", LabelSelector: "app.kubernetes.io/component=memos",
		}))
		checks = append(checks, c.checkPodReady(ctx, PodCheckDef{
			Name: "Memos PG", LabelSelector: "app.kubernetes.io/component=memos-pg",
		}))
	} else {
		checks = append(checks, CheckResult{Name: "Memos", Status: StatusSkip, Detail: "disabled in Helm values"})
		checks = append(checks, CheckResult{Name: "Memos PG", Status: StatusSkip, Detail: "disabled in Helm values"})
	}

	// Hindsight
	if c.values.HindsightEnabled {
		checks = append(checks, c.checkPodReady(ctx, PodCheckDef{
			Name: "Hindsight", LabelSelector: "app.kubernetes.io/component=hindsight",
		}))
		checks = append(checks, c.checkServiceEndpoint(ctx, EndpointCheckDef{
			Name: "Hindsight API", ServiceName: c.cfg.Release + "-hindsight", Port: "8888", Path: "/metrics",
		}))
		checks = append(checks, c.checkPodReady(ctx, PodCheckDef{
			Name: "Hindsight PG", LabelSelector: "app.kubernetes.io/component=hindsight-pg",
		}))
	} else {
		checks = append(checks, CheckResult{Name: "Hindsight", Status: StatusSkip, Detail: "disabled in Helm values"})
		checks = append(checks, CheckResult{Name: "Hindsight API", Status: StatusSkip, Detail: "disabled in Helm values"})
		checks = append(checks, CheckResult{Name: "Hindsight PG", Status: StatusSkip, Detail: "disabled in Helm values"})
	}

	// Ollama
	if c.values.OllamaEnabled {
		checks = append(checks, c.checkPodReady(ctx, PodCheckDef{
			Name: "Ollama", LabelSelector: "app.kubernetes.io/component=ollama",
		}))
		checks = append(checks, c.checkServiceEndpoint(ctx, EndpointCheckDef{
			Name: "Ollama API", ServiceName: c.cfg.Release + "-ollama", Port: "11434", Path: "/api/tags",
		}))
	} else {
		checks = append(checks, CheckResult{Name: "Ollama", Status: StatusSkip, Detail: "disabled in Helm values"})
		checks = append(checks, CheckResult{Name: "Ollama API", Status: StatusSkip, Detail: "disabled in Helm values"})
	}

	return SectionResult{Title: "Memory Services", Icon: "🧠", Checks: checks}
}

// runIngressTLSSection checks ingress controller and TLS certificate.
func (c *checker) runIngressTLSSection(ctx context.Context) SectionResult {
	checks := []CheckResult{
		c.checkPodReady(ctx, PodCheckDef{
			Name: "Traefik", LabelSelector: "app.kubernetes.io/name=traefik",
		}),
		c.checkPodReady(ctx, PodCheckDef{
			Name: "cert-manager", LabelSelector: "app.kubernetes.io/name=cert-manager",
		}),
		c.checkTLSCertificate(ctx),
	}
	return SectionResult{Title: "Ingress & TLS", Icon: "🌐", Checks: checks}
}

// runPlatformSection checks platform service pods and Consul KV.
func (c *checker) runPlatformSection(ctx context.Context) SectionResult {
	var checks []CheckResult
	for _, def := range platformPods {
		checks = append(checks, c.checkPodReady(ctx, def))
	}
	checks = append(checks, c.checkConsulKV(ctx))
	return SectionResult{Title: "Platform Services", Icon: "🔧", Checks: checks}
}

// runHTTPSEndpointSection checks all HTTPS ingress endpoints.
func (c *checker) runHTTPSEndpointSection(ctx context.Context, lbIP string) SectionResult {
	var checks []CheckResult
	for _, def := range ingressEndpoints {
		if def.EnabledFunc != nil && !def.EnabledFunc(c.values) {
			continue
		}
		checks = append(checks, c.checkHTTPSEndpoint(ctx, def, lbIP))
	}
	title := fmt.Sprintf("Endpoints (https://*.%s  LB: %s)", c.cfg.Domain, lbIP)
	return SectionResult{
		Title:  title,
		Icon:   "🔗",
		Checks: checks,
	}
}
