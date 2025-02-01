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
{{- if .Values.kafka.enabled -}}
{{- printf "%s-kafka:9092" (include "meteroid.fullname" .) -}}
{{- else -}}
{{- required "global.kafka.bootstrapServers is required when kafka.enabled=false" .Values.global.kafka.bootstrapServers -}}
{{- end -}}
{{- end }}

{{/*
Get Clickhouse address
*/}}
{{- define "meteroid.clickhouse.address" -}}
{{- if .Values.clickhouse.enabled -}}
{{- printf "tcp://%s-clickhouse:9000" (include "meteroid.fullname" .) -}}
{{- else -}}
{{- required "global.clickhouse.address is required when clickhouse.enabled=false" .Values.global.clickhouse.address -}}
{{- end -}}
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
{{ include "meteroid.fullname" . }}-kafka:9092
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

{{- define "meteroid.schedulerFullname" -}}
{{- printf "%s-scheduler" (include "meteroid.fullname" .) -}}
{{- end }}

{{- define "meteroid.schedulerLabels" -}}
{{- include "meteroid.labels" . }}
app.kubernetes.io/component: scheduler
{{- end }}

{{- define "meteroid.schedulerSelectorLabels" -}}
{{- include "meteroid.selectorLabels" . }}
app.kubernetes.io/component: scheduler
{{- end }}
