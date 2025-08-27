import { ConnectionMetadataItem } from "@/rpc/api/connectors/v1/models_pb";

export function getLatestConnMeta(
  items?: ConnectionMetadataItem[]
): ConnectionMetadataItem | undefined {
  return items?.reduce<ConnectionMetadataItem | undefined>(
    (latest, item) =>
      !latest || new Date(item.syncAt) > new Date(latest.syncAt) ? item : latest,
    undefined
  );
}
