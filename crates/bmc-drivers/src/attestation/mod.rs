/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Hardware attestation capability drivers.

mod nvidia;
mod standard;

pub(crate) use nvidia::HgxAttestation;
pub(crate) use standard::StandardAttestation;
