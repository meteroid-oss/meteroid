import { Aggregation_AggregationType } from '@/rpc/api/billablemetrics/v1/models_pb'

/** Only sum and count split into parts that add up to the whole, so only they have shares. */
export const isAdditiveAggregation = (type: Aggregation_AggregationType | undefined) =>
  type === Aggregation_AggregationType.SUM || type === Aggregation_AggregationType.COUNT
