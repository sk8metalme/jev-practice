#!/bin/sh
set -eu

usage() {
  cat <<'USAGE'
Usage: setup.sh --scope user|project [--repo PATH] [--hooks]

Builds jevx and installs its advisory Codex Skill for the selected scope.
With --hooks, also installs the jevx shadow and compact-assist Codex hooks.
USAGE
}

scope=""
repo="$(pwd)"
install_hooks=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --scope)
      [ "$#" -ge 2 ] || { usage >&2; exit 2; }
      scope="$2"
      shift 2
      ;;
    --repo)
      [ "$#" -ge 2 ] || { usage >&2; exit 2; }
      repo="$2"
      shift 2
      ;;
    --hooks)
      install_hooks=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

case "$scope" in
  user)
    destination="${HOME:?HOME is required}/.agents/skills/jevx"
    ;;
  project)
    destination="$repo/.agents/skills/jevx"
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

if [ "$scope" = "project" ]; then
  repo=$(CDPATH= cd -- "$repo" && pwd)
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

cargo install --path "$project_root" --locked
mkdir -p "$destination"
install -m 0644 "$project_root/skill/SKILL.md" "$destination/SKILL.md"
printf 'Installed jevx and Codex Skill at %s\n' "$destination"

if [ "$install_hooks" -eq 1 ]; then
  cargo_home="${CARGO_HOME:-${HOME:?HOME is required}/.cargo}"
  binary="$cargo_home/bin/jevx"
  "$binary" hooks install --scope "$scope" --repo "$repo"
fi
