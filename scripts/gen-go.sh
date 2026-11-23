#!/usr/bin/env bash
# Regenerates go/settingspb from rust/botconf-proto/proto/ (the proto lives in
# the botconf-proto crate). Run from the repository root.
#
# The committed files were generated with protoc 34.1, protoc-gen-go 1.36.11
# and protoc-gen-go-grpc 1.6.2. Another protoc only changes the version line
# in the files' headers; CI ignores that line when it checks for a diff.
set -euo pipefail
protoc --proto_path=rust/botconf-proto/proto \
  --go_out=go --go_opt=module=github.com/propheci/botconf/go \
  --go-grpc_out=go --go-grpc_opt=module=github.com/propheci/botconf/go \
  rust/botconf-proto/proto/botconf/v1/settings.proto
