## DX scripts

This directory contains scripts that are used to automate or simplify the DX process.

### new_migration.sh

This script is an optional helper to create a new postgres migration.

It uses [atlasgo](https://atlasgo.io/) to generate the SQL DDL for the database based on the existing migrations, then
generate the migration file for any change you make in the DDL.

Make sure to install atlas, then :

- Run ./scripts/new_migration.sh
- Perform any changes as needed.
- Confirm in the terminal and give a migration name in snake_case.
  It will generate the migration file and open it.
- Confirm again to move the migration to the migrations directory and clean temporary files.
