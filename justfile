bin := "unity-sprite-author"
cli_pkg := "unity-sprite-author-cli"
target_dir := justfile_directory() / "target/release"
install_dir := env_var('HOME') / ".local/bin"

default:
    @just --list

# Build the release binary and symlink it into ~/.local/bin.
install:
    cargo build --release -p {{cli_pkg}}
    mkdir -p {{install_dir}}
    ln -sf {{target_dir}}/{{bin}} {{install_dir}}/{{bin}}
    @echo "installed: {{install_dir}}/{{bin}} -> {{target_dir}}/{{bin}}"
    @{{install_dir}}/{{bin}} --help | head -3

uninstall:
    rm -f {{install_dir}}/{{bin}}
    @echo "removed: {{install_dir}}/{{bin}}"
