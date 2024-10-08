### Slurm collector

Small service running on the Slurm controller node that collects information about the jobs running on the cluster and
sends it to the Meteroid ingestion server.

We're using the accounting infrastructure (requires slurmdbd) with simple checkpointing and batching, allowing for
historical data retrieval, low overhead and high flexibility.

Alternatively, we could rely on the job completion infrastructure instead (JobCompType=jobcomp/kafka) for low-latency
use cases.

We currently only retrieve job state, duration, requested CPUs and memory. We could easily extend this to retrieve more.

Some sacct documentation : https://jhpce.jhu.edu/slurm/tips-sacct/
