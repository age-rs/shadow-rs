use crate::build::{ConstType, ConstVal};
use crate::ci::CiType;
use crate::env::{new_project, new_system_env};
use crate::gen_const::{
    clap_long_version_branch_const, clap_long_version_tag_const, version_branch_const,
    version_tag_const, BUILD_CONST_CLAP_LONG_VERSION, BUILD_CONST_VERSION,
};
use crate::git::new_git;
use crate::{
    get_std_env, BuildPattern, DateTime, SdResult, ShadowBuilder, ShadowConst,
    CARGO_CLIPPY_ALLOW_ALL, TAG,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;

pub(crate) const DEFINE_SHADOW_RS: &str = "shadow.rs";

/// `shadow-rs` configuration.
///
/// This struct encapsulates the configuration for the `shadow-rs` build process. It allows for fine-grained control over
/// various aspects of the build, including file output, build constants, environment variables, deny lists, and build patterns.
///
/// While it is possible to construct a [`Shadow`] instance manually, it is highly recommended to use the [`ShadowBuilder`] builder pattern structure
/// provided by `shadow-rs`. The builder pattern simplifies the setup process and ensures that all necessary configurations are properly set up,
/// allowing you to customize multiple aspects simultaneously, such as using a denylist and a hook function at the same time.
///
/// # Fields
///
/// * `f`: The file that `shadow-rs` writes build information to. This file will contain serialized build constants and other metadata.
/// * `map`: A map of build constant identifiers to their corresponding `ConstVal`. These are the values that will be written into the file.
/// * `std_env`: A map of environment variables obtained through [`std::env::vars`]. These variables can influence the build process.
/// * `deny_const`: A set of build constant identifiers that should be excluded from the build process. This can be populated via [`ShadowBuilder::deny_const`].
/// * `out_path`: The path where the generated files will be placed. This is usually derived from the `OUT_DIR` environment variable but can be customized via [`ShadowBuilder::out_path`].
/// * `build_pattern`: Determines the strategy for triggering package rebuilds (`Lazy`, `RealTime`, or `Custom`). This affects when Cargo will rerun the build script and can be configured via [`ShadowBuilder::build_pattern`].
///
/// # Example
///
/// ```no_run
/// use std::collections::BTreeSet;
/// use shadow_rs::{ShadowBuilder, BuildPattern, CARGO_TREE, CARGO_METADATA};
///
/// ShadowBuilder::builder()
///    .build_pattern(BuildPattern::RealTime)
///    .deny_const(BTreeSet::from([CARGO_TREE, CARGO_METADATA]))
///    .build().unwrap();
/// ```
///
#[derive(Debug)]
pub struct Shadow {
    /// The file that `shadow-rs` writes build information to.
    ///
    /// This file will contain all the necessary information about the build, including serialized build constants and other metadata.
    pub f: File,

    /// The values of build constants to be written.
    ///
    /// This is a mapping from `ShadowConst` identifiers to their corresponding `ConstVal` objects. Each entry in this map represents a build constant that will be included in the final build.
    pub map: BTreeMap<ShadowConst, ConstVal>,

    /// Build environment variables, obtained through [`std::env::vars`].
    ///
    /// These environment variables can affect the build process and are captured here for consistency and reproducibility.
    pub std_env: BTreeMap<String, String>,

    /// Constants in the deny list, passed through [`ShadowBuilder::deny_const`].
    ///
    /// This set contains build constant identifiers that should be excluded from the build process. By specifying these, you can prevent certain constants from being written into the build file.
    pub deny_const: BTreeSet<ShadowConst>,

    /// The output path where generated files will be placed.
    ///
    /// This specifies the directory where the build script will write its output. It's typically set using the `OUT_DIR` environment variable but can be customized using [`ShadowBuilder::out_path`].
    pub out_path: String,

    /// Determines the strategy for triggering package rebuilds.
    ///
    /// This field sets the pattern for how often the package should be rebuilt. Options include `Lazy`, `RealTime`, and `Custom`, each with its own implications on the build frequency and conditions under which a rebuild is triggered.
    /// It can be configured using [`ShadowBuilder::build_pattern`].
    pub build_pattern: BuildPattern,
}

impl Shadow {
    /// Write the build configuration specified by this [`Shadow`] instance.
    /// The hook function is run as well, allowing it to append to `shadow-rs`'s output.
    pub fn hook<F>(&self, f: F) -> SdResult<()>
    where
        F: Fn(&File) -> SdResult<()>,
    {
        let desc = r#"// Below code generated by project custom from by build.rs"#;
        writeln!(&self.f, "\n{desc}\n")?;
        f(&self.f)?;
        Ok(())
    }

    /// Try to infer the CI system that we're currently running under.
    ///
    /// TODO: Recognize other CI types, especially Travis and Jenkins.
    fn try_ci(&self) -> CiType {
        if let Some(c) = self.std_env.get("GITLAB_CI") {
            if c == "true" {
                return CiType::Gitlab;
            }
        }

        if let Some(c) = self.std_env.get("GITHUB_ACTIONS") {
            if c == "true" {
                return CiType::Github;
            }
        }

        CiType::None
    }

    /// Checks if the specified build constant is in the deny list.
    ///
    /// # Arguments
    /// * `deny_const` - A value of type `ShadowConst` representing the build constant to check.
    ///
    /// # Returns
    /// * `true` if the build constant is present in the deny list; otherwise, `false`.
    pub fn deny_contains(&self, deny_const: ShadowConst) -> bool {
        self.deny_const.contains(&deny_const)
    }

    pub(crate) fn build_inner(builder: ShadowBuilder) -> SdResult<Shadow> {
        let out_path = builder.get_out_path()?;
        let src_path = builder.get_src_path()?;
        let build_pattern = builder.get_build_pattern().clone();
        let deny_const = builder.get_deny_const().clone();

        let out = {
            let path = Path::new(out_path);
            if !out_path.ends_with('/') {
                path.join(format!("{out_path}/{DEFINE_SHADOW_RS}"))
            } else {
                path.join(DEFINE_SHADOW_RS)
            }
        };

        let mut shadow = Shadow {
            f: File::create(out)?,
            map: Default::default(),
            std_env: Default::default(),
            deny_const,
            out_path: out_path.to_string(),
            build_pattern,
        };
        shadow.std_env = get_std_env();

        let ci_type = shadow.try_ci();
        let src_path = Path::new(src_path.as_str());

        let mut map = new_git(src_path, ci_type, &shadow.std_env);
        for (k, v) in new_project(&shadow.std_env) {
            map.insert(k, v);
        }
        for (k, v) in new_system_env(&shadow) {
            map.insert(k, v);
        }
        shadow.map = map;

        // deny const
        shadow.filter_deny();

        shadow.write_all()?;

        // handle hook
        if let Some(h) = builder.get_hook() {
            shadow.hook(h.hook_inner())?
        }

        Ok(shadow)
    }

    fn filter_deny(&mut self) {
        self.deny_const.iter().for_each(|x| {
            self.map.remove(&**x);
        })
    }

    fn write_all(&mut self) -> SdResult<()> {
        self.gen_header()?;

        self.gen_const()?;

        //write version function
        let gen_version = self.gen_version()?;

        self.gen_build_in(gen_version)?;

        Ok(())
    }

    fn gen_const(&mut self) -> SdResult<()> {
        let out_dir = &self.out_path;
        self.build_pattern.rerun_if(self.map.keys(), out_dir);

        for (k, v) in self.map.clone() {
            self.write_const(k, v)?;
        }
        Ok(())
    }

    fn gen_header(&self) -> SdResult<()> {
        let desc = format!(
            r#"// Code automatically generated by `shadow-rs` (https://github.com/baoyachi/shadow-rs), do not edit.
// Author: https://www.github.com/baoyachi
// Generation time: {}
"#,
            DateTime::now().to_rfc2822()
        );
        writeln!(&self.f, "{desc}\n\n")?;
        Ok(())
    }

    fn write_const(&mut self, shadow_const: ShadowConst, val: ConstVal) -> SdResult<()> {
        let desc = format!("#[doc=r#\"{}\"#]", val.desc);
        let define = match val.t {
            ConstType::Str => format!(
                "#[allow(dead_code)]\n\
                {}\n\
            pub const {} :{} = r#\"{}\"#;",
                CARGO_CLIPPY_ALLOW_ALL,
                shadow_const.to_ascii_uppercase(),
                ConstType::Str,
                val.v
            ),
            ConstType::Bool => format!(
                "#[allow(dead_code)]\n\
            	{}\n\
            pub const {} :{} = {};",
                CARGO_CLIPPY_ALLOW_ALL,
                shadow_const.to_ascii_uppercase(),
                ConstType::Bool,
                val.v.parse::<bool>().unwrap()
            ),
            ConstType::Slice => format!(
                "#[allow(dead_code)]\n\
            	{}\n\
            pub const {} :{} = &{:?};",
                CARGO_CLIPPY_ALLOW_ALL,
                shadow_const.to_ascii_uppercase(),
                ConstType::Slice,
                val.v.as_bytes()
            ),
        };

        writeln!(&self.f, "{desc}")?;
        writeln!(&self.f, "{define}\n")?;
        Ok(())
    }

    fn gen_version(&mut self) -> SdResult<Vec<&'static str>> {
        let (ver_fn, clap_long_ver_fn) = match self.map.get(TAG) {
            None => (version_branch_const(), clap_long_version_branch_const()),
            Some(tag) => {
                if !tag.v.is_empty() {
                    (version_tag_const(), clap_long_version_tag_const())
                } else {
                    (version_branch_const(), clap_long_version_branch_const())
                }
            }
        };
        writeln!(&self.f, "{ver_fn}\n")?;
        writeln!(&self.f, "{clap_long_ver_fn}\n")?;

        Ok(vec![BUILD_CONST_VERSION, BUILD_CONST_CLAP_LONG_VERSION])
    }

    fn gen_build_in(&self, gen_const: Vec<&'static str>) -> SdResult<()> {
        let mut print_val = String::from("\n");

        // append gen const
        for (k, v) in &self.map {
            let tmp = match v.t {
                ConstType::Str | ConstType::Bool => {
                    format!(r#"{}println!("{k}:{{{k}}}\n");{}"#, "\t", "\n")
                }
                ConstType::Slice => {
                    format!(r#"{}println!("{k}:{{:?}}\n",{});{}"#, "\t", k, "\n",)
                }
            };
            print_val.push_str(tmp.as_str());
        }

        // append gen fn
        for k in gen_const {
            let tmp = format!(r#"{}println!("{k}:{{{k}}}\n");{}"#, "\t", "\n");
            print_val.push_str(tmp.as_str());
        }

        #[cfg(not(feature = "no_std"))]
        {
            let everything_define = format!(
                "/// Prints all built-in `shadow-rs` build constants to standard output.\n\
            #[allow(dead_code)]\n\
            {CARGO_CLIPPY_ALLOW_ALL}\n\
            pub fn print_build_in() {\
            {{print_val}}\
            }\n",
            );

            writeln!(&self.f, "{everything_define}")?;

            use crate::gen_const::cargo_metadata_fn;
            writeln!(&self.f, "{}", cargo_metadata_fn(self))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CARGO_TREE;
    use std::fs;

    #[test]
    fn test_build() -> SdResult<()> {
        ShadowBuilder::builder()
            .src_path("./")
            .out_path("./")
            .build()?;
        let shadow = fs::read_to_string(DEFINE_SHADOW_RS)?;
        assert!(!shadow.is_empty());
        assert!(shadow.lines().count() > 0);
        Ok(())
    }

    #[test]
    fn test_build_deny() -> SdResult<()> {
        ShadowBuilder::builder()
            .src_path("./")
            .out_path("./")
            .deny_const(BTreeSet::from([CARGO_TREE]))
            .build()?;

        let shadow = fs::read_to_string(DEFINE_SHADOW_RS)?;
        assert!(!shadow.is_empty());
        assert!(shadow.lines().count() > 0);
        // println!("{shadow}");
        let expect = "pub const CARGO_TREE :&str";
        assert!(!shadow.contains(expect));
        Ok(())
    }

    #[test]
    fn test_env() {
        for (k, v) in std::env::vars() {
            println!("K:{k},V:{v}");
        }
    }
}
