#!/bin/bash
cd roles/isuride/files/webapp
cargo zigbuild --target x86_64-unknown-linux-gnu.2.39 --release
