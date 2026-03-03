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
	"os"
	"time"

	"helm.sh/helm/v3/pkg/action"
	helmchart "helm.sh/helm/v3/pkg/chart"
	"helm.sh/helm/v3/pkg/chart/loader"
	"helm.sh/helm/v3/pkg/cli"
	helmrelease "helm.sh/helm/v3/pkg/release"
	"k8s.io/apimachinery/pkg/api/meta"
	"k8s.io/client-go/discovery"
	"k8s.io/client-go/discovery/cached/memory"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/restmapper"
	"k8s.io/client-go/tools/clientcmd"
	clientcmdapi "k8s.io/client-go/tools/clientcmd/api"
)

const defaultHelmTimeout = 15 * time.Minute

// HelmManager manages helm chart installations for a given namespace.
type HelmManager struct {
	kubeconfigPath string
	namespace      string
	settings       *cli.EnvSettings
}

// NewHelmManager creates a HelmManager using the given kubeconfig file.
func NewHelmManager(kubeconfigPath, namespace string) *HelmManager {
	settings := cli.New()
	settings.KubeConfig = kubeconfigPath
	return &HelmManager{
		kubeconfigPath: kubeconfigPath,
		namespace:      namespace,
		settings:       settings,
	}
}

// InstallOrUpgrade installs a helm chart if not present, or upgrades it if already installed.
func (m *HelmManager) InstallOrUpgrade(ctx context.Context, releaseName, chartName, version, repoURL string, values map[string]interface{}) error {
	cfg, err := m.newActionConfig()
	if err != nil {
		return err
	}

	chrt, err := m.pullChart(chartName, version, repoURL)
	if err != nil {
		return fmt.Errorf("pull chart %s@%s: %w", chartName, version, err)
	}

	histClient := action.NewHistory(cfg)
	histClient.Max = 1
	history, histErr := histClient.Run(releaseName)

	tctx, cancel := context.WithTimeout(ctx, defaultHelmTimeout)
	defer cancel()

	// Determine if the last release is in a failed/pending state and needs reinstall.
	needsReinstall := histErr == nil && len(history) > 0 && isFailedRelease(history[0])

	if histErr != nil || needsReinstall {
		if needsReinstall {
			// Clean up the stuck release before reinstalling.
			uninstall := action.NewUninstall(cfg)
			uninstall.Timeout = defaultHelmTimeout
			uninstall.Wait = true
			_, _ = uninstall.Run(releaseName) // best effort; ignore error
		}
		// Fresh install
		client := action.NewInstall(cfg)
		client.ReleaseName = releaseName
		client.Namespace = m.namespace
		client.CreateNamespace = true
		client.Timeout = defaultHelmTimeout
		client.Wait = true
		client.WaitForJobs = true
		if _, err := client.RunWithContext(tctx, chrt, values); err != nil {
			return fmt.Errorf("helm install %s: %w", releaseName, err)
		}
	} else {
		// Upgrade existing healthy release
		client := action.NewUpgrade(cfg)
		client.Namespace = m.namespace
		client.Timeout = defaultHelmTimeout
		client.Wait = true
		client.WaitForJobs = true
		client.ReuseValues = false
		client.MaxHistory = 3
		if _, err := client.RunWithContext(tctx, releaseName, chrt, values); err != nil {
			return fmt.Errorf("helm upgrade %s: %w", releaseName, err)
		}
	}

	return nil
}

// isFailedRelease reports whether a release is in a state that requires
// uninstalling before reinstalling (failed, pending-install, pending-upgrade).
func isFailedRelease(r *helmrelease.Release) bool {
	switch r.Info.Status {
	case helmrelease.StatusFailed,
		helmrelease.StatusPendingInstall,
		helmrelease.StatusPendingUpgrade,
		helmrelease.StatusPendingRollback:
		return true
	}
	return false
}

// pullChart uses helm pull to download and load a chart from the given repo.
func (m *HelmManager) pullChart(chartName, version, repoURL string) (*helmchart.Chart, error) {
	tmpDir, err := os.MkdirTemp("", "rara-helm-*")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(tmpDir)

	emptyCfg := new(action.Configuration)
	pull := action.NewPullWithOpts(action.WithConfig(emptyCfg))
	pull.Settings = m.settings
	pull.ChartPathOptions.RepoURL = repoURL
	pull.ChartPathOptions.Version = version
	pull.DestDir = tmpDir
	pull.Untar = true
	pull.UntarDir = tmpDir

	if _, err := pull.Run(chartName); err != nil {
		return nil, fmt.Errorf("pull %s from %s: %w", chartName, repoURL, err)
	}

	chartPath := fmt.Sprintf("%s/%s", tmpDir, chartName)
	return loader.Load(chartPath)
}

// newActionConfig creates an action.Configuration backed by the kubeconfig file.
func (m *HelmManager) newActionConfig() (*action.Configuration, error) {
	cfg := new(action.Configuration)
	getter := &kubeconfigGetter{
		kubeconfigPath: m.kubeconfigPath,
		namespace:      m.namespace,
	}
	if err := cfg.Init(getter, m.namespace, "secret", func(format string, v ...interface{}) {
		// suppress helm debug logs
	}); err != nil {
		return nil, fmt.Errorf("init helm action config: %w", err)
	}
	return cfg, nil
}

// kubeconfigGetter implements genericclioptions.RESTClientGetter using a kubeconfig file.
type kubeconfigGetter struct {
	kubeconfigPath string
	namespace      string
	mapper         meta.RESTMapper
}

func (g *kubeconfigGetter) ToRESTConfig() (*rest.Config, error) {
	return clientcmd.BuildConfigFromFlags("", g.kubeconfigPath)
}

func (g *kubeconfigGetter) ToDiscoveryClient() (discovery.CachedDiscoveryInterface, error) {
	rc, err := g.ToRESTConfig()
	if err != nil {
		return nil, err
	}
	dc, err := discovery.NewDiscoveryClientForConfig(rc)
	if err != nil {
		return nil, err
	}
	return memory.NewMemCacheClient(dc), nil
}

func (g *kubeconfigGetter) ToRESTMapper() (meta.RESTMapper, error) {
	if g.mapper != nil {
		return g.mapper, nil
	}
	dc, err := g.ToDiscoveryClient()
	if err != nil {
		return nil, err
	}
	g.mapper = restmapper.NewDeferredDiscoveryRESTMapper(dc)
	return g.mapper, nil
}

func (g *kubeconfigGetter) ToRawKubeConfigLoader() clientcmd.ClientConfig {
	return clientcmd.NewNonInteractiveDeferredLoadingClientConfig(
		&clientcmd.ClientConfigLoadingRules{ExplicitPath: g.kubeconfigPath},
		&clientcmd.ConfigOverrides{
			Context: clientcmdapi.Context{Namespace: g.namespace},
		},
	)
}
