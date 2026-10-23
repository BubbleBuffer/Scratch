// The "Run schedules on this device" setting. Desktop only (the add-on
// registers the section only there). It is stored on this device, never in
// the vault, and turning it on or off starts or stops this computer's runner.
// The Add-ons panel is gone, so the section is placed by the settings screen
// that renders [ScheduleRunnerSwitch] (its UI lane).

import 'package:flutter/widgets.dart';

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

/// The state behind the switch. The saved flag changes only after the runner
/// has changed: a failure keeps the old flag and sets [error], which the UI
/// shows under the switch until the next change succeeds.
class ScheduleRunnerSwitch extends ChangeNotifier {
  ScheduleRunnerSwitch({required this.settings, required this.runner});

  /// Where the flag is saved (this device's settings).
  final AppSettings settings;

  /// Starts (true) or stops (false) this device's runner. Returns the failure
  /// as text, or null when it worked, like [setLocalRunner]. A throw counts as
  /// a failure too.
  final Future<String?> Function(bool on) runner;

  /// The saved flag: whether this computer runs schedules.
  bool get on => settings.scheduleRunsHere;

  /// The last failure to change the runner, or null.
  String? get error => _error;
  String? _error;

  /// True while a change waits for the runner. [set] is ignored meanwhile.
  bool get busy => _busy;
  bool _busy = false;

  bool _disposed = false;

  /// Turns the runner on or off. Returns true when the runner changed and the
  /// flag was saved, false when it failed (see [error]) or a change is pending.
  /// The flag is saved even if the controller was disposed meanwhile; only the
  /// notification is skipped.
  Future<bool> set(bool on) async {
    if (_busy) return false;
    _busy = true;
    _notify();
    String? failure;
    try {
      failure = await runner(on);
    } catch (e) {
      failure = e.toString().trim();
    }
    _busy = false;
    _error = failure;
    if (failure == null) settings.update((s) => s.scheduleRunsHere = on);
    _notify();
    return failure == null;
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}

class _RunnerSetting extends StatefulWidget {
  const _RunnerSetting({required this.services});
  final AddonServices services;

  @override
  State<_RunnerSetting> createState() => _RunnerSettingState();
}

class _RunnerSettingState extends State<_RunnerSetting> {
  ScheduleRunnerSwitch? _switch;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final vault = widget.services.vault(context);
    _switch ??= ScheduleRunnerSwitch(
      settings: widget.services.settings(context),
      runner: (on) => setLocalRunner(vault, on),
    );
  }

  @override
  void dispose() {
    _switch?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final runner = _switch!;
    final colors = context.colors;
    return ListenableBuilder(
      listenable: runner,
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
              value: runner.on,
              onChanged: runner.busy ? null : (on) => runner.set(on),
            ),
          ),
          if (runner.error != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                'The runner did not change: ${runner.error}',
                key: const ValueKey('schedule-runner-error'),
                style: tsRaw(13, 18, 450, colors.textSecondary),
              ),
            ),
        ],
      ),
    );
  }
}
