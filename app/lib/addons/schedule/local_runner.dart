// This computer's schedule runner, as the schedule add-on turns it on and off.
// Desktop only: Android has no runner, so the add-on neither registers the
// setting there nor calls the vault's runner calls.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../core/vault.dart';

/// True on the platforms that run schedules on this device (Windows, Linux
/// and macOS).
bool get localRunnerPlatform => switch (defaultTargetPlatform) {
  TargetPlatform.windows || TargetPlatform.linux || TargetPlatform.macOS =>
    true,
  _ => false,
};

/// Vaults whose runner has been started with the stored setting this session.
final Expando<bool> _started = Expando<bool>();

/// Sets this device's runner on or off for [vault]. Returns the failure as
/// text for the caller to show, or null when it worked. Does nothing off
/// desktop.
Future<String?> setLocalRunner(Vault vault, bool available) async {
  if (!localRunnerPlatform) return null;
  try {
    await vault.localRunnerSetAvailable(available);
    return null;
  } catch (e) {
    return e.toString().trim();
  }
}

/// Starts [vault]'s runner with the stored setting, once per vault and
/// session. The first schedule backend for a vault calls it; see the wiki for
/// why this is not at app start.
void startLocalRunnerOnce(Vault vault, bool available) {
  if (!localRunnerPlatform || _started[vault] == true) return;
  _started[vault] = true;
  unawaited(setLocalRunner(vault, available));
}
