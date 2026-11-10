//! One module per subcommand.

pub mod bench;
pub mod compare;
pub mod convert;
pub mod info;
pub mod list;
pub mod run;
pub mod validate;

use model_registry::{BuildRequest, Built, ParamOverride, Registry, RegistryError};

use crate::cli::BuildArgs;

/// The build request of `target` with the shared `--seed` / `--param` options.
///
/// # Errors
/// Malformed `key=value` overrides.
pub fn request(target: &str, args: &BuildArgs) -> anyhow::Result<BuildRequest> {
    let params = args.params.iter().map(|p| ParamOverride::parse(p)).collect::<Result<Vec<_>, _>>()?;
    Ok(BuildRequest { target: target.to_string(), seed: args.seed, params })
}

/// Builds a simulation; unknown targets mention skipped scenario files.
///
/// # Errors
/// Registry errors with context (model crate errors already name the
/// scenario being built).
pub fn build(registry: &Registry, request: &BuildRequest) -> anyhow::Result<Built> {
    registry.build(request).map_err(|e| {
        let skipped = registry.problems().len();
        let unknown = matches!(e, RegistryError::UnknownTarget { .. });
        let names_target = matches!(e, RegistryError::Build { .. });
        let mut error = anyhow::Error::new(e);
        if unknown && skipped > 0 {
            error = error.context(format!(
                "{skipped} scenario file(s) were skipped because they are invalid; `emergence-cli list scenarios` \
                 lists them"
            ));
        }
        if names_target { error } else { error.context(format!("cannot build `{}`", request.target)) }
    })
}
