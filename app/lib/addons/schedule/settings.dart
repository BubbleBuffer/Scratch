// The "Run schedules on this device" setting, in Settings > Add-ons under
// Schedules. Desktop only (the add-on registers it only there). It is stored
// on this device, never in the vault, and turning it on or off starts or
// stops this computer's runner.

import 'package:flutter/widgets.dart';

import '../../core/vault.dart';
import '../../registry/addon_registry.dart';
import '../../services/addon_services.dart';
import '../../ui/ui.dart';
import 'local_runner.dart';

/// The settings section for this device's runner.
AddonSettingsSection scheduleRunnerSetting(AddonServices services) =>
    AddonSettingsSection(
      id: 'schedule.runner',
      label: 'Schedules',
      build: (context) => _RunnerSetting(services: services),
    );

class _RunnerSetting extends StatefulWidget {
  const _RunnerSetting({required this.services});
  final AddonServices services;

  @override
  State<_RunnerSetting> createState() => _RunnerSettingState();
}

class _RunnerSettingState extends State<_RunnerSetting> {
  /// The last failure to start or stop the runner, shown under the switch.
  String? _error;

  Future<void> _set(Vault vault, AppSettings settings, bool on) async {
    settings.update((s) => s.scheduleRunsHere = on);
    final failure = await setLocalRunner(vault, on);
    if (mounted) setState(() => _error = failure);
  }

  @override
  Widget build(BuildContext context) {
    final vault = widget.services.vault(context);
    final settings = widget.services.settings(context);
    final colors = context.colors;
    return ListenableBuilder(
      listenable: settings,
      builder: (context, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SettingsRow(
            label: 'Run schedules on this device',
            description:
                'This computer runs schedules while the app is open. '
                'Off by default.',
            trailing: Toggle(
              key: const ValueKey('schedule-runs-here'),
              value: settings.scheduleRunsHere,
              onChanged: (on) => _set(vault, settings, on),
            ),
          ),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                'The runner did not change: $_error',
                key: const ValueKey('schedule-runner-error'),
                style: tsRaw(13, 18, 450, colors.textSecondary),
              ),
            ),
        ],
      ),
    );
  }
}
