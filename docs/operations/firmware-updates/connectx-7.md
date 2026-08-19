# ConnectX-7 Host InfiniBand Firmware Update

Use this procedure to update ConnectX-7 (CX7) firmware on GPU hosts before an operation, such as a UFM upgrade, that requires a newer host InfiniBand firmware baseline.

This is a managed-host firmware update performed through Scout. It is not a [DPU NIC update](dpu-firmware.md) and does not use the rack/tray `cx7` target.

## Define the rollout baseline

Obtain the approved firmware release and record these values before changing any environment:

| Requirement                              | Approved value             |
| ---------------------------------------- | -------------------------- |
| Adapter PSID                             | `<approved-psid>`          |
| Expected ConnectX-7 adapters per host    | `<expected-adapter-count>` |
| Target firmware version                  | `<target-version>`         |
| Firmware image SHA-256                   | `<image-sha256>`           |
| Scout script SHA-256 for a legacy bundle | `<script-sha256>`          |

NICo does not choose the latest firmware or validate the approved PSID and adapter count. Those values are release inputs that the operator must verify.

## Confirm that the host model is supported

The legacy workflow is catalog-driven. It can update a host model when its effective firmware catalog contains:

- the exact BMC vendor and model;
- a `cx7` component with a matcher for every ConnectX-7 firmware inventory entry;
- an approved firmware image and Scout script; and
- `cx7` in the model's update `ordering`.

The presence of ConnectX-7 hardware alone is not sufficient. If `firmware show` does not list a CX7 target for the host's vendor and model, stop and have the platform owner publish and validate a catalog definition for that model.

## Publish with the packaged Scout script

Use this method when `firmware show` lists CX7 for the exact host vendor and model and the release includes a packaged CX7 Scout script for that model.

1. Publish the approved firmware image at an HTTPS URL reachable from the target hosts.
2. Calculate the image's lowercase SHA-256 digest.
3. Use the [Host Firmware Config API](configuration.md#configure-host-firmware-through-the-api) to set the `Cx7` component's target, mark it as the default, provide the image URL and digest, and preserve the model's complete update ordering.
4. Confirm the effective target with `firmware show`.

Do not provide a script path or script checksum in the firmware request. NICo selects the packaged script by vendor, model, and component, calculates its digest, and uses the timeouts shipped beside that script. PXE serves it at:

```text
<pxe-base-url>/public/scout-firmware-scripts/<vendor>/<model>/cx7/upgrade.sh
```

Changing the packaged script or its timeouts requires a NICo release. Updating only the firmware container or API entry does not change the script.

With the packaged script and a URL-based firmware entry, the firmware container contains only `metadata.toml`. Scout downloads the image from the URL in that metadata, and NICo supplies the script. Older legacy containers can still contain the image and script for deployments that require them.

> **Limitation:** On deployments using packaged script resolution, the legacy firmware entry's `scout` block does not select a script or set its timeouts. If no packaged CX7 script exists for the exact vendor and model, do not use the new method.

## Publish the legacy catalog and files

The legacy firmware container must contain:

```text
<vendor>-<model>-cx7-<target-version>/
|-- metadata.toml
|-- <firmware-image>
`-- scripts/
    `-- <upgrade-script>
```

Its `metadata.toml` must define the exact BMC vendor and model, the complete component ordering, the CX7 inventory matcher, one default target, the activation requirement, and the paths, SHA-256 digests, and timeouts for the image and Scout script.

A non-empty ordering in newer legacy metadata replaces the older model ordering. Omitting an existing component can prevent its normal firmware workflow from running.

Publish the same immutable bundle separately to:

1. Core's configured firmware directory, so NICo can load `metadata.toml`; and
2. the environment's PXE firmware tree, so Scout can download the script and image through `/public/firmware/<bundle>/...`.

Deploy the same firmware container to Core and PXE through the environment's existing firmware-container process. The container copies its files into the shared firmware volume. Do not edit files in a running pod, and pin the container by its immutable digest.

From the target host network, download the exact PXE URLs and compare their SHA-256 values with `metadata.toml`:

```bash
curl --fail --output /tmp/cx7-image \
  "http://<pxe-host>/public/firmware/<bundle>/<firmware-image>"
curl --fail --output /tmp/cx7-upgrade-script \
  "http://<pxe-host>/public/firmware/<bundle>/scripts/<upgrade-script>"
sha256sum /tmp/cx7-image /tmp/cx7-upgrade-script
```

Confirm the effective target before requesting an update:

```bash
nico-admin-cli -a <core-api-url> firmware show
```

The output must contain the intended vendor, model, component `CX7`, and target version.

## Update an unassigned host

Start with one unassigned host in `Ready`:

```bash
nico-admin-cli -a <core-api-url> host reprovision set \
  --id <machine-id> \
  --update-message "<ticket-or-maintenance-reference>"
```

This direct request bypasses automatic selection and the Machine Update Manager concurrency calculation. Control rollout concurrency in the maintenance plan.

No instance action, instance deletion, machine deletion, or return to pre-ingestion is required. NICo prevents allocation, runs the Scout update, performs the configured activation power operation, verifies inventory, and returns the host to `Ready`.

## Update a host assigned to an instance

Create the same host reprovisioning request:

```bash
nico-admin-cli -a <core-api-url> host reprovision set \
  --id <machine-id> \
  --update-message "<ticket-or-maintenance-reference>"
```

The request remains pending until the tenant approves the disruptive reboot or the environment enters its configured automatic reboot period. To approve it:

```bash
nico-admin-cli -a <core-api-url> instance reboot \
  --instance <instance-id> \
  --apply-updates-on-reboot
```

NICo reboots the instance, starts Scout, updates the adapters, performs activation, and returns the host to the assigned-instance state.

Do not delete the instance or machine. `HostReprovision` is the internal firmware workflow name; it does not mean that the instance association is removed. Refer to [Assigned hosts need reboot approval](host-firmware.md#assigned-hosts-need-reboot-approval).

## Monitor the update

```bash
nico-admin-cli -a <core-api-url> host reprovision list
nico-admin-cli -a <core-api-url> managed-host show <machine-id>
nico-admin-cli -a <core-api-url> machine show <machine-id> --history-count 10
```

The principal states are:

| State | Meaning |
| --- | --- |
| `CheckingFirmwareV2` | NICo compares all matching CX7 inventory versions with the target. |
| `WaitingForScoutUpgrade` | Scout is downloading, verifying, and running the update. |
| `ResetForNewFirmware` | NICo is performing the configured activation operation. |
| `NewFirmwareReportedWait` | NICo is waiting for refreshed BMC inventory. |
| `FailedFirmwareUpgrade` | The script, download, activation, or verification failed. |

## Verify completion

Do not treat a successful script exit as completion. Verify all of the following:

1. The fresh Site Explorer report contains the expected number of CX7 entries.
2. Every CX7 entry reports the target firmware version.
3. An authorized hardware inventory or MFT query still reports the approved PSID on every adapter.
4. The host returned to `Ready` or `Assigned/Ready`.
5. `host reprovision list` no longer contains the machine.

Only after all impacted hosts pass these checks should the dependent UFM operation proceed.

## Failures and recovery

| Symptom | Check |
| --- | --- |
| No CX7 entry in `firmware show` | Check the exact vendor/model, metadata location, default version, and complete ordering. |
| No update is selected | Check the inventory matcher and confirm at least one reported version differs from the target. |
| Adapter count differs from the rollout baseline | Stop. Reconcile hardware inventory before flashing. NICo does not enforce the expected count. |
| PSID differs from the approved baseline | Stop. Do not use the image unless the release owner approves that PSID. |
| Scout cannot download a file | Fetch the exact PXE URL from the host network and check the published path. |
| Checksum mismatch | Recalculate the downloaded file's SHA-256 and compare it with the catalog. |
| No eligible ConnectX-7 device | Confirm that the image contains firmware for the adapters and approved PSID. |
| Assigned host remains pending | Confirm the request exists and obtain reboot approval, or check the automatic reboot period. |
| Inventory does not converge | Compare every matching CX7 entry with the exact target and inspect activation and BMC inventory. |

After correcting the underlying problem, restart firmware checking:

```bash
nico-admin-cli -a <core-api-url> managed-host \
  reset-host-reprovisioning --machine <machine-id>
```

Refer to [Managed Host Firmware Updates](host-firmware.md#failures-and-recovery) for retry timing, metrics, and additional recovery guidance.
