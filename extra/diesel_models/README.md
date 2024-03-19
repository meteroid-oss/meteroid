https://diesel.rs/guides/getting-started.html

cargo install diesel_cli --no-default-features --features postgres

diesel print-schema > schema.rs



cargo install diesel_cli_ext

-------------------------

- HistoryGenerator
  - SetupGenerator
  - RepartitionGenerator
  - DayGenerator
- LiveGenerator


We'll talk about the history generator.
It may be run punctually to create a Base Seed, and then just for the remaining day until today, then day by day.

We start with simple parameters, about the initial state and wanted final state.
The generator then handles the distribution, and mùakes sure to have thing realistic and production-like.

In a next version, we can also add more "checkpoints" to the history.
This will allow to generate random environments that are closely similar to production environments 
(without data masking or anything, we just use anonymous subscription metrics)


Flow: 

The HistoryGenerator is creating realistic data from parametors across a specific period

We start with the v1 of all plans, at a specific day


/////////////////

Step 1 : 
- create the customers with subscriptions with a proper repartition over time

Step 2 :
- create the invoices

Step 3 : 
- Add trial management with some randomness

Step 4 : 
- Add upgrades/downgrades/churns

Step 5 :
- Add new versions

Step 6 : 
- Add migrations

Step X : 
- Add the usage data

