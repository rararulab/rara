//! Integration tests for PodManager against a real k3s cluster.
//!
//! All tests share a single k3s container (expensive to start) and run
//! sequentially inside one `#[tokio::test]` function to keep the tokio
//! runtime alive for the lifetime of the kube client.

use std::collections::BTreeMap;
use std::time::Duration;

use k8s_openapi::api::core::v1 as k8s_core;
use kube::api::ObjectMeta;
use kube::config::{KubeConfigOptions, Kubeconfig};
use rara_k8s::{K8sError, PodManager};
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::k3s::{K3s, KUBE_SECURE_PORT};

// ---------------------------------------------------------------------------
// Cluster setup
// ---------------------------------------------------------------------------

async fn setup_cluster() -> (PodManager, testcontainers::ContainerAsync<K3s>) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rara_k8s=debug,kube=info")
        .with_test_writer()
        .try_init();

    // Ensure rustls crypto provider is installed (required by kube).
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install rustls crypto provider");
    }

    let conf_dir = tempfile::tempdir().expect("failed to create temp dir");

    let k3s = K3s::default()
        .with_conf_mount(conf_dir.path())
        .with_privileged(true)
        .with_userns_mode("host");

    let container = k3s.start().await.expect("failed to start k3s container");

    // Read kubeconfig written by k3s.
    let conf_yaml = container
        .image()
        .read_kube_config()
        .expect("failed to read k3s kubeconfig");

    // Rewrite the server URL to use the mapped host port.
    let port = container
        .get_host_port_ipv4(KUBE_SECURE_PORT)
        .await
        .expect("failed to get k3s port");

    let mut kubeconfig =
        Kubeconfig::from_yaml(&conf_yaml).expect("failed to parse kubeconfig yaml");

    kubeconfig.clusters.iter_mut().for_each(|cluster| {
        if let Some(server) = cluster
            .cluster
            .as_mut()
            .and_then(|c| c.server.as_mut())
        {
            *server = format!("https://127.0.0.1:{port}");
        }
    });

    let mut client_config =
        kube::Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
            .await
            .expect("failed to build kube config");

    // Disable proxy -- we connect directly to the local k3s container.
    // Without this, kube picks up HTTPS_PROXY from the environment and
    // fails with ProxyProtocolDisabled when the http-proxy feature is off.
    client_config.proxy_url = None;

    let client =
        kube::Client::try_from(client_config).expect("failed to create kube client");

    let manager = PodManager::with_client(client);

    // Leak the tempdir so it lives as long as the container.
    std::mem::forget(conf_dir);

    (manager, container)
}

// ---------------------------------------------------------------------------
// Pod builder helpers
// ---------------------------------------------------------------------------

/// Build a simple nginx pod for testing.
fn nginx_pod(name: &str) -> k8s_core::Pod {
    k8s_core::Pod {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            labels: Some(BTreeMap::from([(
                "app".to_string(),
                "test".to_string(),
            )])),
            ..Default::default()
        },
        spec: Some(k8s_core::PodSpec {
            restart_policy: Some("Never".to_string()),
            containers: vec![k8s_core::Container {
                name: "nginx".to_string(),
                image: Some("nginx:alpine".to_string()),
                ports: Some(vec![k8s_core::ContainerPort {
                    container_port: 80,
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            ..Default::default()
        }),
        status: None,
    }
}

/// Build a busybox pod that echoes and sleeps.
fn echo_pod(name: &str) -> k8s_core::Pod {
    k8s_core::Pod {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            ..Default::default()
        },
        spec: Some(k8s_core::PodSpec {
            restart_policy: Some("Never".to_string()),
            containers: vec![k8s_core::Container {
                name: "echo".to_string(),
                image: Some("busybox:latest".to_string()),
                command: Some(vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "echo hello-from-pod && sleep 300".to_string(),
                ]),
                ..Default::default()
            }],
            ..Default::default()
        }),
        status: None,
    }
}

// ---------------------------------------------------------------------------
// Test suite -- all subtests in one function to share the k3s cluster and
// keep the tokio runtime alive.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pod_manager_integration() {
    let (manager, _container) = setup_cluster().await;

    // -- subtest: create and delete pod ---------------------------------
    {
        println!(">> test_create_and_delete_pod");

        let pod = nginx_pod("test-create-delete");
        let handle = manager
            .create_pod(pod, "default", Duration::from_secs(120))
            .await
            .expect("failed to create pod");

        assert_eq!(handle.name, "test-create-delete");
        assert_eq!(handle.namespace, "default");
        assert!(handle.ip.is_some(), "pod should have an IP");
        assert_eq!(handle.port, Some(80));

        manager
            .delete_pod(&handle.name, &handle.namespace)
            .await
            .expect("failed to delete pod");

        println!("   PASSED");
    }

    // -- subtest: get pod status ----------------------------------------
    {
        println!(">> test_get_pod_status");

        let pod = nginx_pod("test-status");
        let handle = manager
            .create_pod(pod, "default", Duration::from_secs(120))
            .await
            .expect("failed to create pod");

        let status = manager
            .get_pod_status(&handle.name, "default")
            .await
            .expect("failed to get pod status");

        assert_eq!(status.phase, "Running");
        assert!(status.ready);
        assert!(status.ip.is_some());

        manager
            .delete_pod(&handle.name, "default")
            .await
            .expect("failed to delete pod");

        println!("   PASSED");
    }

    // -- subtest: get pod logs ------------------------------------------
    {
        println!(">> test_get_pod_logs");

        let pod = echo_pod("test-logs");
        let handle = manager
            .create_pod(pod, "default", Duration::from_secs(120))
            .await
            .expect("failed to create pod");

        // Give the container a moment to produce output.
        tokio::time::sleep(Duration::from_secs(2)).await;

        let logs = manager
            .get_pod_logs(&handle.name, "default", Some(10))
            .await
            .expect("failed to get pod logs");

        assert!(
            logs.contains("hello-from-pod"),
            "logs should contain echo output, got: {logs}"
        );

        manager
            .delete_pod(&handle.name, "default")
            .await
            .expect("failed to delete pod");

        println!("   PASSED");
    }

    // -- subtest: delete nonexistent pod is ok --------------------------
    {
        println!(">> test_delete_nonexistent_pod_is_ok");

        let result = manager
            .delete_pod("nonexistent-pod-12345", "default")
            .await;

        assert!(result.is_ok(), "delete of non-existent pod should be Ok");

        println!("   PASSED");
    }

    // -- subtest: create pod timeout with bad image ---------------------
    {
        println!(">> test_create_pod_timeout_with_bad_image");

        let pod = k8s_core::Pod {
            metadata: ObjectMeta {
                name: Some("test-bad-image".to_string()),
                ..Default::default()
            },
            spec: Some(k8s_core::PodSpec {
                restart_policy: Some("Never".to_string()),
                containers: vec![k8s_core::Container {
                    name: "bad".to_string(),
                    image: Some("this-image-does-not-exist:never".to_string()),
                    image_pull_policy: Some("Always".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: None,
        };

        // Use short timeout to trigger timeout error.
        let result = manager
            .create_pod(pod, "default", Duration::from_secs(15))
            .await;

        assert!(result.is_err(), "should fail with bad image");

        match result.unwrap_err() {
            K8sError::PodTimeout { name, .. } => {
                assert_eq!(name, "test-bad-image");
            }
            other => panic!("expected PodTimeout, got: {other:?}"),
        }

        println!("   PASSED");
    }
}
