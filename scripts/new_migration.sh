#!/bin/bash


SCRIPT_DIR="$(dirname $(realpath $0))"
BASE_DIR="$(dirname $SCRIPT_DIR)"

MODULE_PATH="$BASE_DIR/modules/meteroid/crates/meteroid-migrations"
TEMP_DIR="_migrations"

# Set current directory to module path
cd $MODULE_PATH

# Clean up and prepare
rm -rf $TEMP_DIR
mkdir $TEMP_DIR


cleanup() {
    echo -e "\nCaught CTRL+C. Cleaning up..."
    rm -rf "$TEMP_DIR"
    [[ -f "$NEXT_FILE_NAME" ]] && rm "$NEXT_FILE_NAME"
    exit 1
}

trap cleanup SIGINT

echo "Loading schema"
# Run initial commands
atlas migrate import --from file://refinery/migrations --dir-format=flyway --to file://$TEMP_DIR/migrations
atlas migrate hash --dir file://$TEMP_DIR/migrations
atlas schema inspect -u file://$TEMP_DIR/migrations --dev-url "docker://postgres/15?search_path=public" --format '{{ sql . }}' > $TEMP_DIR/schema.sql

# Prompt user for input and wait
echo "Please update the $TEMP_DIR/schema.sql file. Once done, enter the migration name and press [ENTER]..."

if [ "$TERM_PROGRAM" = "vscode" ]; then
  # If the user is using Visual Studio Code, open the file with 'code' command
  code "$MODULE_PATH/$TEMP_DIR/schema.sql" || true
elif [ "$TERMINAL_EMULATOR" = "JetBrains-JediTerm" ]; then
  # If the user is using IntelliJ IDEA's JediTerm, open the file with 'idea' command
  rustrover "$MODULE_PATH/$TEMP_DIR/schema.sql" || idea "$MODULE_PATH/$TEMP_DIR/schema.sql" || true
fi


read -p "Migration Name: " MIGRATION_NAME

# generate the migration
while :; do
    atlas migrate diff "$MIGRATION_NAME" \
      --dir "file://$TEMP_DIR/migrations/?search_path=public" \
      --to "file://$TEMP_DIR/schema.sql/?search_path=public" \
      --dev-url "docker://postgres/15?search_path=public" || {
        echo "Atlas diff failed. Please fix the error and press [ENTER]..."
        echo "Success"
        continue
    }
    break
done

cd "$TEMP_DIR/migrations"

NEW_MIGRATION_FILE=$(ls | grep "${MIGRATION_NAME}.sql")

if [[ -z "$NEW_MIGRATION_FILE" ]]; then
    echo "No change to apply."
    cd ../..
    rm -rf "$TEMP_DIR"
    exit
fi

HIGHEST_NUMBER=$(ls | grep -E "^[0-9]{4}_.*\.sql$" | sort -n | tail -n 1 | cut -d"_" -f1)
NEXT_NUMBER=$((10#${HIGHEST_NUMBER} + 1))
NEXT_FILE_NAME="V$(printf "%04d" $NEXT_NUMBER)__${MIGRATION_NAME}.sql"

cp "$NEW_MIGRATION_FILE" "../$NEXT_FILE_NAME"
cd ../..

echo "New migration created: $NEXT_FILE_NAME"

rm -rf "$TEMP_DIR/migrations"

if [ "$TERM_PROGRAM" = "vscode" ]; then
  # If the user is using Visual Studio Code, open the file with 'code' command
  code "$MODULE_PATH/$TEMP_DIR/$NEXT_FILE_NAME" || true
elif [ "$TERMINAL_EMULATOR" = "JetBrains-JediTerm" ]; then
  # If the user is using IntelliJ IDEA's JediTerm, open the file with 'idea' command
  rustrover "$MODULE_PATH/$TEMP_DIR/$NEXT_FILE_NAME" || idea "$MODULE_PATH/$TEMP_DIR/$NEXT_FILE_NAME" || true
fi

read -p "Is it valid? If yes, it will be moved to the refinery folder. (y/n): " is_valid

[[ "$is_valid" == "y" ]] && mv "$TEMP_DIR/$NEXT_FILE_NAME" "$MODULE_PATH/refinery/migrations/" && echo "Migration moved to refinery folder." || echo "Reverting."

rm -rf "$TEMP_DIR"

