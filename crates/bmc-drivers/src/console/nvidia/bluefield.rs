/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use super::super::support::{DPU_SSH_PORT, DirectSshConsole};

/// NVIDIA BlueField: the DPU console is an SSH service on its own port.
pub(crate) static BLUEFIELD_CONSOLE: DirectSshConsole = DirectSshConsole {
    port: DPU_SSH_PORT,
    message: "DPU console is directly available over SSH",
};
