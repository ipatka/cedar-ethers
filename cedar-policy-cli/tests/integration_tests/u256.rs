/*
 * Copyright 2022-2023 Amazon.com, Inc. or its affiliates. All Rights Reserved.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Integration tests targeting the decimal extension
//!
//! These files exist separately in the `CedarIntegrationTests` package

use super::perform_integration_test_from_json;
use std::path::Path;

/// Path of the folder containing the JSON tests
fn folder() -> &'static Path {
    Path::new("tests/u256")
}

#[test]
fn u256_1() {
    perform_integration_test_from_json(folder().join("1.json"));
}

#[test]
fn u256_2() {
    perform_integration_test_from_json(folder().join("2.json"));
}
