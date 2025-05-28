use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Trait for test instances that can be used with golden tests
pub trait TestInstances {
    /// Returns a vector of named test instances
    fn instances() -> Vec<(String, Self)>
    where
        Self: Sized;
}

/// Main struct for running golden tests
pub struct GoldenTest<T> {
    manifest_dir: PathBuf,
    _phantom: PhantomData<T>,
}

impl<T> Default for GoldenTest<T>
where
    T: TestInstances + serde::Serialize + serde::de::DeserializeOwned,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> GoldenTest<T>
where
    T: TestInstances + serde::Serialize + serde::de::DeserializeOwned,
{
    /// Create a new golden test instance
    #[inline]
    pub fn new() -> Self {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| panic!("CARGO_MANIFEST_DIR not set. Are you running with cargo?"));
        Self {
            manifest_dir,
            _phantom: PhantomData,
        }
    }

    /// Run the golden test with the specified folder name
    pub fn run(&self, folder_name: &str) {
        let test_folder = self.golden_folder(folder_name);

        if Self::is_update_mode() {
            self.update_golden_files(&test_folder);
        } else {
            self.verify_golden_files(&test_folder);
        }
    }

    /// Update golden files with new test instances
    fn update_golden_files(&self, test_folder: &Path) {
        fs::create_dir_all(test_folder).expect("Failed to create golden test directory");

        let instances = T::instances();
        let existing_content = self.read_existing_golden_files(test_folder);
        let version_tag = Self::get_version_tag();

        for (name, instance) in instances {
            let json =
                serde_json::to_string_pretty(&instance).expect("Failed to serialize test instance");

            // Skip if content already exists
            if existing_content
                .get(&name)
                .is_some_and(|contents| contents.contains(&json))
            {
                println!(
                    "Golden file for '{}' already exists with identical content",
                    name
                );
                continue;
            }

            let file_path = test_folder.join(format!("{}_{}.json", version_tag, name));
            File::create(&file_path)
                .and_then(|mut file| file.write_all(json.as_bytes()))
                .unwrap_or_else(|e| {
                    panic!("Failed to write golden file {}: {}", file_path.display(), e)
                });

            println!("Updated golden file: {}", file_path.display());
        }
    }

    /// Read existing golden files from the test folder
    fn read_existing_golden_files(&self, test_folder: &Path) -> HashMap<String, Vec<String>> {
        let mut existing_content = HashMap::new();

        if !test_folder.exists() {
            return existing_content;
        }

        for entry in fs::read_dir(test_folder).expect("Failed to read golden test directory") {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    eprintln!("Warning: Could not read directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();

            // Skip non-JSON files
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }

            // Extract the variant name from the filename (part after the '_')
            let filename = match path.file_name().and_then(|f| f.to_str()) {
                Some(name) => name,
                None => continue,
            };

            let variant_name = match filename.find('_').zip(filename.rfind('.')) {
                Some((underscore_pos, dot_pos)) => &filename[underscore_pos + 1..dot_pos],
                None => continue,
            };

            // Read file content
            if let Ok(content) = fs::read_to_string(&path) {
                existing_content
                    .entry(variant_name.to_string())
                    .or_insert_with(Vec::new)
                    .push(content);
            } else {
                eprintln!("Warning: Couldn't read file {}", path.display());
            }
        }

        existing_content
    }

    /// Verify golden files against test instances
    fn verify_golden_files(&self, test_folder: &Path) {
        if !test_folder.exists() {
            panic!(
                "No golden files found in {}. Try running with UPDATE_GOLDEN=1",
                test_folder.display()
            );
        }

        let mut tested_any = false;

        for entry in fs::read_dir(test_folder).expect("Failed to read golden test directory") {
            let path = entry.expect("Failed to read directory entry").path();

            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }

            tested_any = true;
            let filename = path.file_name().unwrap_or_default();

            let json_str = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("Failed to read golden file {}: {}", path.display(), e));

            let _deserialized: T = serde_json::from_str(&json_str).unwrap_or_else(|e| {
                panic!(
                    "Failed to deserialize from {}: {}",
                    filename.to_string_lossy(),
                    e
                )
            });

            println!("Successfully tested {}", filename.to_string_lossy());
        }

        assert!(tested_any, "No golden test files found");
    }

    #[inline]
    fn golden_folder(&self, name: &str) -> PathBuf {
        self.manifest_dir.join("tests").join("golden").join(name)
    }

    #[inline]
    fn is_update_mode() -> bool {
        env::var("UPDATE_GOLDEN").is_ok_and(|v| v == "1")
    }

    /// Get a timestamp-based version tag to avoid collisions
    #[inline]
    fn get_version_tag() -> String {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        format!("{:08x}", t - 10e8 as u64)
    }
}

/// Macro for creating golden tests
#[macro_export]
macro_rules! golden {
    // Version that uses type name as folder name
    ($type:ty, { $($name:expr => $value:expr),* $(,)? }) => {
        paste::paste! {
            impl golden::TestInstances for $type {
                fn instances() -> Vec<(String, Self)> {
                    vec![
                        $(
                            ($name.to_string(), $value),
                        )*
                    ]
                }
            }

            #[test]
            fn [<golden_test_ $type:snake>]() {
                let test = golden::GoldenTest::<$type>::new();
                test.run(&stringify!($type).to_lowercase());
            }
        }
    };

    // Version with custom folder name
    ($type:ty, $folder:expr, { $($name:expr => $value:expr),* $(,)? }) => {
        paste::paste! {
            impl golden::TestInstances for $type {
                fn instances() -> Vec<(String, Self)> {
                    vec![
                        $(
                            ($name.to_string(), $value),
                        )*
                    ]
                }
            }

            #[test]
            fn [<golden_test_ $type:snake>]() {
                let test = golden::GoldenTest::<$type>::new();
                test.run($folder);
            }
        }
    };
}
