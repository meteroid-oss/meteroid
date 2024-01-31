#!/bin/bash

cd "$(dirname "$0")"

rm -rf ./generated
mkdir -p ./generated

ROOT_NODE_MODULES="$(cd ../ && pwd)/node_modules"

export PATH=$PATH:$ROOT_NODE_MODULES/.bin

# protoc --experimental_allow_proto3_optional \
#   --es_out generated \
#   --es_opt target=ts \
#   --connect-query_out generated \
#   --connect-query_opt target=ts \
#   --connect-query_opt import_extension=none \
#  --proto_path=../../.. \
#  -I=../../meteroid/proto/ ../../meteroid/proto/plans/*.proto ../../meteroid/proto/*.proto

PROTO_BASE_PATH=../../meteroid
PROTO_COMMON_BASE_PATH=../../../crates/common-grpc

protoc --experimental_allow_proto3_optional \
  --es_out generated \
  --es_opt target=ts \
  --connect-query_out generated \
  --connect-query_opt target=ts \
  --connect-query_opt import_extension=none \
  --proto_path=$PROTO_COMMON_BASE_PATH/proto \
  -I=$PROTO_COMMON_BASE_PATH/proto \
  $PROTO_COMMON_BASE_PATH/proto/common/v1/*.proto

protoc --experimental_allow_proto3_optional \
  --es_out generated \
  --es_opt target=ts \
  --connect-query_out generated \
  --connect-query_opt target=ts \
  --connect-query_opt import_extension=none \
  --proto_path=../../.. \
  -I=$PROTO_COMMON_BASE_PATH/proto \
  -I=$PROTO_BASE_PATH/proto \
  $PROTO_BASE_PATH/proto/api/apitokens/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/billablemetrics/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/customers/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/instance/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/invoices/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/plans/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/pricecomponents/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/productfamilies/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/products/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/schedules/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/subscriptions/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/shared/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/tenants/v1/*.proto \
  $PROTO_BASE_PATH/proto/api/users/v1/*.proto 
