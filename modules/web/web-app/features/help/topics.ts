/** The help page opened on a topic contributed to the `help.topics` slot. */
export const helpTopicHref = (basePath: string, topic: string) =>
  `${basePath}/help?topic=${encodeURIComponent(topic)}`
