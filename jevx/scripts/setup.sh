#!/bin/sh
set -eu

usage() {
  cat <<'USAGE'
Usage: setup.sh --scope user|project [--repo PATH]

Builds jevx and installs its advisory Codex Skill for the selected scope.
USAGE
}

scope=""
repo="$(pwd)"
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

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

cargo install --path "$project_root" --locked
mkdir -p "$destination"
install -m 0644 "$project_root/skill/SKILL.md" "$destination/SKILL.md"
printf 'Installed jevx and Codex Skill at %s\n' "$destination"
