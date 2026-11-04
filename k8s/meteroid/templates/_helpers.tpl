{{/*
Expand the name of the chart.
*/}}
{{- define "meteroid.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "meteroid.fullname" -}}
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
{{- define "meteroid.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "meteroid.labels" -}}
helm.sh/chart: {{ include "meteroid.chart" . }}
{{ include "meteroid.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "meteroid.selectorLabels" -}}
app.kubernetes.io/name: {{ include "meteroid.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}


 
{{/*
Create the name of the service account to use
*/}}
{{- define "meteroid.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
    {{ default (include "meteroid.fullname" .) .Values.serviceAccount.name }}
{{- else -}}
    {{ default "default" .Values.serviceAccount.name }}
{{- end -}}
{{- end -}}


{{/*
Get the secrets name
*/}}
{{- define "meteroid.secretsName" -}}
{{- .Values.global.secrets.existingSecretName | default (printf "%s-secrets" (include "meteroid.fullname" .)) -}}
{{- end }}

{{/*
Get Kafka address
*/}}
{{- define "meteroid.kafka.bootstrapServers" -}}
{{- required "global.kafka.bootstrapServers is required" .Values.global.kafka.bootstrapServers -}}
{{- end }}

{{/*
Get internal service URLs
*/}}
{{- define "meteroid.api.internalGrpcUrl" -}}
{{ include "meteroid.apiFullname" . }}:{{ .Values.api.service.ports.grpc }}
{{- end }}

{{- define "meteroid.metering.internalGrpcUrl" -}}
{{ include "meteroid.meteringFullname" . }}:{{ .Values.metering.service.ports.grpc }}
{{- end }}

{{- define "meteroid.kafka.internalUrl" -}}
{{ include "meteroid.fullname" . }}-kafka-kafka-bootstrap:9092
{{- end }}

{{/*
Component-specific helpers
*/}}

{{- define "meteroid.apiFullname" -}}
{{- printf "%s-api" (include "meteroid.fullname" .) -}}
{{- end }}

{{- define "meteroid.apiLabels" -}}
{{- include "meteroid.labels" . }}
app.kubernetes.io/component: api
{{- end }}

{{- define "meteroid.apiSelectorLabels" -}}
{{- include "meteroid.selectorLabels" . }}
app.kubernetes.io/component: api
{{- end }}

{{- define "meteroid.webFullname" -}}
{{- printf "%s-web" (include "meteroid.fullname" .) -}}
{{- end }}

{{- define "meteroid.webLabels" -}}
{{- include "meteroid.labels" . }}
app.kubernetes.io/component: web
{{- end }}

{{- define "meteroid.webSelectorLabels" -}}
{{- include "meteroid.selectorLabels" . }}
app.kubernetes.io/component: web
{{- end }}

{{- define "meteroid.meteringFullname" -}}
{{- printf "%s-metering" (include "meteroid.fullname" .) -}}
{{- end }}

{{- define "meteroid.meteringLabels" -}}
{{- include "meteroid.labels" . }}
app.kubernetes.io/component: metering
{{- end }}

{{- define "meteroid.meteringSelectorLabels" -}}
{{- include "meteroid.selectorLabels" . }}
app.kubernetes.io/component: metering
{{- end }}

{{- define "meteroid.workerFullname" -}}
{{- printf "%s-worker" (include "meteroid.fullname" .) -}}
{{- end }}

{{- define "meteroid.workerLabels" -}}
{{- include "meteroid.labels" . }}
app.kubernetes.io/component: worker
{{- end }}

{{- define "meteroid.workerSelectorLabels" -}}
{{- include "meteroid.selectorLabels" . }}
app.kubernetes.io/component: worker
{{- end }}

{{/*
The worker's values: `worker`, overridden by the former `scheduler` key, read until 2.1.
*/}}
{{- define "meteroid.workerValues" -}}
{{- toYaml (mergeOverwrite (deepCopy .Values.worker) (deepCopy (.Values.scheduler | default dict))) }}
{{- end }}

{{/*
Settings shared by the API and the worker
*/}}
{{- define "meteroid.settingsEnv" -}}
{{- with .Values.global.mfaIssuer }}
- name: METEROID_MFA_ISSUER
  value: {{ . | quote }}
{{- end }}
{{- $webhooks := .Values.global.webhooks | default dict }}
{{- $webhookEnv := dict
  "deliveryConcurrency" "WEBHOOK_DELIVERY_CONCURRENCY"
  "allowPrivateDestinations" "WEBHOOK_ALLOW_PRIVATE_DESTINATIONS"
  "egressProxyUrl" "WEBHOOK_EGRESS_PROXY_URL"
  "retentionDays" "WEBHOOK_RETENTION_DAYS"
  "ssoAllowPrivateDestinations" "EGRESS_ALLOW_PRIVATE_DESTINATIONS_SSO"
  "ssoUseProxy" "EGRESS_USE_PROXY_SSO" }}
{{- range $key, $name := $webhookEnv }}
{{- if and (hasKey $webhooks $key) (ne (toString (index $webhooks $key)) "") }}
- name: {{ $name }}
  value: {{ index $webhooks $key | quote }}
{{- end }}
{{- end }}
{{- end }}
