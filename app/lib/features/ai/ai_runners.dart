// The runner calls the AI tasks make: list the runners (the hub, then each
// PC), assign a task to one of them, and rename a PC. The add-on picker and
// Settings reach these through the services barrel; tests pass a fake.

import '../../core/vault.dart';
import '../../src/rust/api/ai.dart';

abstract interface class AiRunnerBridge {
  /// The hub (when [hubUsable]) and the vault's PCs, with the default marked.
  Future<AiRunnerList> listRunners({required bool hubUsable});

  /// Queues a task about [page] pinned to [runnerId], offline runners too.
  Future<AiTask> assign({
    required String prompt,
    required String page,
    required String runnerId,
  });

  /// Renames the device [deviceId] to [name] (trimmed by the vault).
  Future<void> rename({required String deviceId, required String name});
}

/// The runner calls on the open vault.
class VaultAiRunnerBridge implements AiRunnerBridge {
  const VaultAiRunnerBridge(this.vault);

  final Vault vault;

  @override
  Future<AiRunnerList> listRunners({required bool hubUsable}) =>
      vault.run((v) => aiListRunners(vault: v, hubUsable: hubUsable));

  @override
  Future<AiTask> assign({
    required String prompt,
    required String page,
    required String runnerId,
  }) => vault.run(
    (v) => aiAssignTask(
      vault: v,
      prompt: prompt,
      page: page,
      runnerId: runnerId,
    ),
    writes: true,
  );

  @override
  Future<void> rename({required String deviceId, required String name}) =>
      vault.run(
        (v) => aiRenameDevice(vault: v, deviceId: deviceId, name: name),
        writes: true,
      );
}
