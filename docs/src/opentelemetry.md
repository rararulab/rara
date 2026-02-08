# OpenTelemetry Integration

This document describes how to use the OpenTelemetry integration in `rsketch` to monitor the performance of the HTTP and gRPC servers.

## Configuration

The OpenTelemetry integration is configured through environment variables. The following variables are available:

* `OTEL_EXPORTER_OTLP_ENDPOINT`: The endpoint of the OpenTelemetry collector. Defaults to `http://localhost:4317`.
* `OTEL_SERVICE_NAME`: The name of the service. Defaults to `rsketch`.

## Usage

To enable the OpenTelemetry integration, simply start the `rsketch` server:

```bash
cargo run --bin rsketch -- server
```

The server will automatically start exporting traces and metrics to the configured OpenTelemetry collector.

## Kubernetes Integration

To integrate with the Kubernetes infrastructure described in the `k8s/` directory, you will need to deploy an OpenTelemetry collector to your cluster. The collector can be configured to export data to a variety of backends, such as Jaeger, Prometheus, or a cloud-based observability platform.

Here is an example of a simple OpenTelemetry collector configuration that exports data to Jaeger:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: otel-collector-conf
  labels:
    app: opentelemetry
    component: otel-collector-conf
data:
  otel-collector-config: |
    receivers:
      otlp:
        protocols:
          grpc:
          http:

    processors:
      batch:

    exporters:
      jaeger:
        endpoint: jaeger-all-in-one:14250
        tls:
          insecure: true

    service:
      pipelines:
        traces:
          receivers: [otlp]
          processors: [batch]
          exporters: [jaeger]
```

This configuration creates a ConfigMap that contains the OpenTelemetry collector configuration. The collector is configured to receive data over OTLP (gRPC and HTTP) and export it to a Jaeger instance running in the cluster.

To deploy the collector, you can use the following Kubernetes manifest:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: otel-collector
  labels:
    app: opentelemetry
    component: otel-collector
spec:
  replicas: 1
  selector:
    matchLabels:
      app: opentelemetry
      component: otel-collector
  template:
    metadata:
      labels:
        app: opentelemetry
        component: otel-collector
    spec:
      containers:
      - name: otel-collector
        image: otel/opentelemetry-collector:0.84.0
        command:
          - "--config=/conf/otel-collector-config.yaml"
        volumeMounts:
        - name: otel-collector-config-vol
          mountPath: /conf
        ports:
        - name: otlp-grpc
          containerPort: 4317
        - name: otlp-http
          containerPort: 4318
      volumes:
        - name: otel-collector-config-vol
          configMap:
            name: otel-collector-conf
```

This manifest creates a Deployment that runs the OpenTelemetry collector. The collector is configured to use the ConfigMap created in the previous step.

Once the collector is deployed, you will need to configure the `rsketch` server to export data to the collector. You can do this by setting the `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable to the address of the collector's OTLP gRPC endpoint.

For example, if the collector is running in the `default` namespace, you can set the environment variable to `http://otel-collector.default:4317`.
