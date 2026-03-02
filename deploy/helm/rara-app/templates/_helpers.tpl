{{/*
Expand the name of the chart.
*/}}
{{- define "rara-app.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "rara-app.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "rara-app.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "rara-app.labels" -}}
helm.sh/chart: {{ include "rara-app.chart" . }}
{{ include "rara-app.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: rara-app
{{- end }}

{{/*
Selector labels
*/}}
{{- define "rara-app.selectorLabels" -}}
app.kubernetes.io/name: {{ include "rara-app.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Component labels helper - pass component name as argument
Usage: {{ include "rara-app.componentLabels" (dict "root" . "component" "backend") }}
*/}}
{{- define "rara-app.componentLabels" -}}
helm.sh/chart: {{ include "rara-app.chart" .root }}
app.kubernetes.io/name: {{ .component }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
{{- if .root.Chart.AppVersion }}
app.kubernetes.io/version: {{ .root.Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .root.Release.Service }}
app.kubernetes.io/part-of: rara-app
app.kubernetes.io/component: {{ .component }}
{{- end }}

{{/*
Component selector labels helper
Usage: {{ include "rara-app.componentSelectorLabels" (dict "root" . "component" "backend") }}
*/}}
{{- define "rara-app.componentSelectorLabels" -}}
app.kubernetes.io/name: {{ .component }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
{{- end }}

{{/*
TLS secret name — defaults to {infra.releaseName}-wildcard-tls
*/}}
{{- define "rara-app.tlsSecretName" -}}
{{- if .Values.ingress.tlsSecretName }}
{{- .Values.ingress.tlsSecretName }}
{{- else }}
{{- printf "%s-wildcard-tls" .Values.infra.releaseName }}
{{- end }}
{{- end }}
