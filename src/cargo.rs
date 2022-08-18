use std::{
    io::BufReader,
    process::{Command, Stdio},
};

use cargo_metadata::{
    camino::Utf8PathBuf, Artifact, Message, Metadata, MetadataCommand, Package, Target,
};
use eyre::ensure;

/// Executes a `cargo build` command and returns paths to the build artifacts.
///
/// # Examples
///
/// ```no_run
/// # fn main() -> eyre::Result<()> {
/// // executes cargo build
/// let metadata = cli_xtask::workspace::current();
/// for bin in cli_xtask::cargo::build(metadata, None, None, None, false, None)? {
///     let bin = bin?;
///     println!("{bin}");
/// }
///
/// // executes cross build --profile target --bin foo --target aarch64-unknown-linux-gnu
/// let metadata = cli_xtask::workspace::current();
/// let package = metadata.root_package().unwrap();
/// let target = package.targets.iter().find(|t| t.name == "foo").unwrap();
/// for bin in cli_xtask::cargo::build(metadata, Some(&package), Some(target), Some("release"), true, Some("aarch64-unknown-linux-gnu"))? {
///     let bin = bin?;
///     println!("{bin}");
/// }
/// # Ok(())
/// # }
/// ```
#[tracing::instrument(name = "cargo::build", skip_all, err)]
pub fn build<'a>(
    metadata: &'a Metadata,
    package: Option<&Package>,
    target: Option<&Target>,
    profile: Option<&str>,
    use_cross: bool,
    target_triple: Option<&str>,
) -> eyre::Result<impl Iterator<Item = eyre::Result<Utf8PathBuf>> + 'a> {
    let cmd_name = if use_cross { "cross" } else { "cargo" };
    let mut args = vec!["build"];

    if let Some(package) = package {
        args.extend(["--package", package.name.as_str()]);
    }

    if let Some(target) = target {
        for kind in &target.kind {
            match kind.as_str() {
                "bin" => args.extend(["--bin", target.name.as_str()]),
                "example" => args.extend(["--example", target.name.as_str()]),
                "test" => args.extend(["--test", target.name.as_str()]),
                "bench" => args.extend(["--bench", target.name.as_str()]),
                "lib" => args.extend(["--lib"]),
                _ => eyre::bail!("unsupported target kind: {}", kind),
            }
        }
    }

    if let Some(profile) = profile {
        args.extend(["--profile", profile]);
    }

    if let Some(target_triple) = target_triple {
        args.extend(["--target", target_triple]);
    }

    let cross_target_dir = if use_cross {
        let mut cmd = MetadataCommand::new();
        cmd.cargo_path("cross").no_deps();
        if let Some(target_triple) = target_triple {
            cmd.other_options(["--target".to_string(), target_triple.to_string()]);
        }
        Some(cmd.exec()?.target_directory)
    } else {
        None
    };

    tracing::info!("{} {}", cmd_name, args.join(" "));
    args.push("--message-format=json-render-diagnostics");

    let mut cmd = Command::new(cmd_name);
    cmd.args(&args);

    let mut cmd = cmd.stdout(Stdio::piped()).spawn()?;
    let stdout = cmd.stdout.take().unwrap();

    let reader = BufReader::new(stdout);
    let it = Message::parse_stream(reader)
        .map(|res| res.map_err(eyre::Error::from))
        .filter_map(|res| match res {
            Ok(Message::CompilerArtifact(Artifact { executable, .. })) => executable.map(Ok),
            Err(e) => Some(Err(e)),
            _ => None,
        })
        .map(move |res| {
            res.and_then(|mut exe| {
                if let Some(target_dir) = &cross_target_dir {
                    let relative = exe.strip_prefix(target_dir)?;
                    exe = metadata.target_directory.join(relative);
                }
                ensure!(exe.is_file(), "Artifact is not a file: {exe}");
                Ok(exe)
            })
        });
    Ok(it)
}
