use std::path::Path;

use uuid::Uuid;

use super::RtmHarness;

pub fn write_fake_cli(dir: &Path) {
    let path = dir.join("docker");
    std::fs::write(&path, FAKE_DOCKER).expect("fake docker");
    let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("permissions");
}

pub fn container_pid(harness: &RtmHarness, session_id: Uuid) -> u32 {
    let path = harness
        .temp_path()
        .join("fake-docker-state")
        .join(format!("rtm-{session_id}.pid"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read fake container pid {}: {error}", path.display()))
        .trim()
        .parse()
        .expect("fake container pid")
}

const FAKE_DOCKER: &str = r#"#!/bin/sh
set -eu
state="$(dirname "$0")/fake-docker-state"
mkdir -p "$state"

case "${1:-}" in
  --version)
    printf 'Docker version 27.0.0\n'
    ;;
  version)
    printf 'fake-docker\n'
    ;;
  image)
    printf '"1000"\n'
    ;;
  manifest)
    printf '{"manifests":[{"platform":{"architecture":"arm64"}}]}\n'
    ;;
  run)
    shift
    name=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --name) name="$2"; shift 2 ;;
        --label|--mount|--workdir|--env) shift 2 ;;
        --rm|-d|-i|-t|--init) shift ;;
        --*) shift ;;
        *) shift; break ;;
      esac
    done
    command="$1"; shift
    case "$command" in
      */*) ;;
      *) [ ! -x "$(dirname "$0")/$command" ] || command="$(dirname "$0")/$command" ;;
    esac
    nohup "$command" "$@" > "$state/$name.out" 2>&1 < /dev/null &
    printf '%s\n' "$!" > "$state/$name.pid"
    printf '%s\n' "$name"
    ;;
  attach)
    shift
    while [ "$#" -gt 1 ]; do
      case "$1" in
        --detach-keys) [ "$2" = "" ] || exit 21; shift 2 ;;
        --sig-proxy=false) shift ;;
        *) exit 22 ;;
      esac
    done
    name="$1"
    touch "$state/$name.out"
    tail -n +1 -f "$state/$name.out" &
    tail_pid="$!"
    pid="$(cat "$state/$name.pid")"
    while kill -0 "$pid" 2>/dev/null; do sleep 0.05; done
    kill "$tail_pid" 2>/dev/null || true
    ;;
  container)
    name="$3"
    if kill -0 "$(cat "$state/$name.pid")" 2>/dev/null; then
      printf 'true\n'
    else
      printf 'false\n'
    fi
    ;;
  kill)
    shift
    signal="15"
    if [ "${1:-}" = "--signal" ]; then signal="$2"; shift 2; fi
    name="$1"
    kill "-$signal" "$(cat "$state/$name.pid")" 2>/dev/null || exit 1
    ;;
  *)
    printf 'unexpected docker command: %s\n' "$*" >&2
    exit 64
    ;;
esac
"#;
