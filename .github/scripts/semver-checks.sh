#!/usr/bin/env bash
# Copyright (c) 2026 Devexperts LLC.
# SPDX-License-Identifier: MPL-2.0

# Checks the public API of dxfeed-graal and dxfeed-graal-build against the latest release tag with cargo-semver-checks
# (the SemVer job of build.yml). The feature experimental is outside SemVer, so it is never enabled. Before 1.0.0 a
# breaking change is reported as a warning and the script succeeds; from 1.0.0 on it fails.
set -euo pipefail

baseline=$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' --exclude '*-*' HEAD)
version=$(cargo pkgid -p dxfeed-graal)
version=${version##*[#@]}
major=${version%%.*}

release_type=()
if [[ "v$version" == "$baseline" ]]; then
    if [[ "$major" == 0 ]]; then
        release_type=(--release-type patch)
    else
        release_type=(--release-type minor)
    fi
fi
echo "Version $version against $baseline ${release_type[*]}"

status=0
check() {
    if ! cargo semver-checks --baseline-rev "$baseline" "${release_type[@]}" "$@"; then
        status=1
    fi
}
check -p dxfeed-graal --only-explicit-features
check -p dxfeed-graal --only-explicit-features --features stream
check -p dxfeed-graal-build --only-explicit-features

if [[ "$status" != 0 && "$major" == 0 ]]; then
    echo "::warning title=SemVer::the API breaks that of $baseline; before 1.0.0 the next minor version may do so"
    status=0
fi
exit "$status"
