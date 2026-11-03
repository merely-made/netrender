// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use super::{EXTERNAL_IMAGE_KEY_BASE, external_image_key};

#[test]
fn external_image_key_keeps_producer_keys_in_a_disjoint_namespace() {
    assert_eq!(external_image_key(0), EXTERNAL_IMAGE_KEY_BASE);
    assert_eq!(external_image_key((1_u64 << 62) - 1), u64::MAX,);
}

#[test]
#[should_panic(expected = "lower-62-bit namespace")]
fn external_image_key_rejects_the_first_aliasing_key() {
    let _ = external_image_key(1_u64 << 62);
}
