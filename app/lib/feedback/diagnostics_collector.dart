// Gathers the platform, device and package facts for a feedback report. The
// app's own values (route, page, sync state, settings, page count, logs) come
// in as parameters, so this file depends on no app state class.

import 'dart:io' show Platform;
import 'dart:ui' as ui;

import 'package:device_info_plus/device_info_plus.dart';
import 'package:package_info_plus/package_info_plus.dart';

import '../ui/app_name.dart';
import 'report.dart';

/// Set at build time with --dart-define=GIT_COMMIT=...; empty when absent.
const _gitCommit = String.fromEnvironment('GIT_COMMIT');

Future<Diagnostics> collectDiagnostics({
  required ui.Size screenSize,
  required double pixelRatio,
  required String route,
  required String? pageId,
  required String? pageTitle,
  required String themeSetting,
  required bool effectiveDark,
  required bool signedIn,
  required String? hubUrl,
  required DateTime? lastSync,
  required String? lastError,
  required bool scheduleRunner,
  required int pageCount,
  required List<String> logs,
  required List<String> errors,
}) async {
  final package = await PackageInfo.fromPlatform();
  final device = await DeviceInfoPlugin().deviceInfo;
  final dispatcher = ui.PlatformDispatcher.instance;
  return Diagnostics(
    appName: appName,
    appVersion: package.version,
    buildNumber: package.buildNumber,
    gitCommit: _gitCommit,
    platform: Platform.operatingSystem,
    osVersion: Platform.operatingSystemVersion,
    deviceModel: _deviceModel(device),
    locale: dispatcher.locale.toLanguageTag(),
    screenSize: (screenSize.width, screenSize.height),
    pixelRatio: pixelRatio,
    themeSetting: themeSetting,
    effectiveDark: effectiveDark,
    route: route,
    pageId: pageId,
    pageTitle: pageTitle,
    signedIn: signedIn,
    hubUrl: hubUrl,
    lastSync: lastSync,
    lastError: lastError,
    scheduleRunner: scheduleRunner,
    pageCount: pageCount,
    logs: logs,
    errors: errors,
  );
}

String _deviceModel(BaseDeviceInfo info) => switch (info) {
  AndroidDeviceInfo i => '${i.manufacturer} ${i.model}',
  IosDeviceInfo i => i.utsname.machine,
  MacOsDeviceInfo i => i.model,
  WindowsDeviceInfo i => i.productName,
  LinuxDeviceInfo i => i.prettyName,
  _ => 'unknown',
};
