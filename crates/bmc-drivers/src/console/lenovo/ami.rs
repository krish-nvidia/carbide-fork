/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use super::super::support::{DirectSshConsole, SSH_PORT};

/// Lenovo AMI: SSH login lands on the serial console directly.
pub(crate) static LENOVO_AMI_CONSOLE: DirectSshConsole = DirectSshConsole {
    port: SSH_PORT,
    message: "SSH login opens the serial console directly",
};
