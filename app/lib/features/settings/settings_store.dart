// App settings of this device: theme, vault location, the embedded hub and
// the hub connection. Kept in `<data dir>/settings.json` (per device, not in
// the vault, so they never sync). Read with [AppSettings.of]; without a
// [SettingsScope] above (tests) a shared in-memory instance is used.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';

import '../reminders/reminder_plan.dart';
import '../sync/cert_trust.dart';

enum ThemePref {
  system,
  light,
  dark;

  String get label => switch (this) {
    system => 'Use system setting',
    light => 'Light',
    dark => 'Dark',
  };

  /// NotesShell's forceDark: null follows the system.
  bool? get forceDark => switch (this) {
    system => null,
    light => false,
    dark => true,
  };
}

/// The default hub address: every interface, port 47100.
const defaultHubAddress = '0.0.0.0:47100';

class AppSettings extends ChangeNotifier {
  AppSettings({this.file});

  /// Where the settings are saved (null: in memory only).
  final File? file;

  ThemePref theme = ThemePref.system;

  /// The vault folder chosen in settings (null: the default in the data
  /// folder). Takes effect at the next start.
  String? vaultPath;

  /// Run the embedded hub (desktop). Null means the default: on for desktop.
  bool? _hubEnabled;
  bool get hubEnabled => _hubEnabled ?? true;

  /// Run schedules on this device (desktop). Device-local: it lives in this
  /// device's settings file, never in the vault. Off by default.
  bool scheduleRunsHere = false;

  /// "Let the hub use this computer" (desktop): the hub this device is
  /// signed in to may run this computer's agent tools. Device-local, off by
  /// default.
  bool hubUsesThisComputer = false;

  /// Phones: a shake opens the feedback sheet. Device-local, on by default.
  bool shakeToSend = true;

  /// Windows updates: also offer nightly builds. Off by default.
  bool updatesIncludeNightly = false;

  /// Windows updates: download a new version in the background (it is only
  /// installed on the user's click). On by default.
  bool updatesAutoDownload = true;

  String hubAddress = defaultHubAddress;

  /// The hub this device syncs with, and its pairing token.
  String? clientUrl;
  String? clientToken;

  /// The label returned by browser sign-in (null for LAN pairing).
  String? clientDeviceLabel;

  /// The account returned by the Hub's validated OIDC login. Older Hubs
  /// return only the device label, so this can be absent while signed in.
  String? clientAccount;

  /// Browser sign-in destination, retained after sign-out. Empty until the
  /// user enters their hub's URL: there is no built-in hub.
  String hubSignInUrl = '';

  /// Download every attachment in the background, not only those opened.
  bool prefetchAttachments = false;

  /// Certificates trusted for hubs with an unknown issuer, per origin.
  TrustedCertificates trustedCertificates = TrustedCertificates();

  /// Notification preferences belong to this device, never to the vault.
  ReminderSettings reminders = const ReminderSettings();

  static final AppSettings memory = AppSettings();

  static AppSettings of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<SettingsScope>()?.notifier ??
      memory;

  static AppSettings read(BuildContext context) =>
      context.getInheritedWidgetOfExactType<SettingsScope>()?.notifier ??
      memory;

  /// Loads [file]; missing or unreadable settings give the defaults.
  static AppSettings load(File file) {
    final s = AppSettings(file: file);
    try {
      if (file.existsSync()) {
        s._fromJson(
          jsonDecode(file.readAsStringSync()) as Map<String, dynamic>,
        );
      }
    } catch (_) {}
    return s;
  }

  /// Every add-on is always on, so a legacy `disabledAddons` key is not read,
  /// and [toJson] leaves it out: the next save drops it from the file.
  void _fromJson(Map<String, dynamic> j) {
    theme = ThemePref.values.firstWhere(
      (t) => t.name == j['theme'],
      orElse: () => ThemePref.system,
    );
    vaultPath = j['vaultPath'] as String?;
    _hubEnabled = j['hubEnabled'] as bool?;
    scheduleRunsHere = (j['scheduleRunsHere'] as bool?) ?? false;
    hubUsesThisComputer = (j['hubUsesThisComputer'] as bool?) ?? false;
    shakeToSend = (j['shakeToSend'] as bool?) ?? true;
    updatesIncludeNightly = (j['updatesIncludeNightly'] as bool?) ?? false;
    updatesAutoDownload = (j['updatesAutoDownload'] as bool?) ?? true;
    hubAddress = (j['hubAddress'] as String?) ?? defaultHubAddress;
    clientUrl = j['clientUrl'] as String?;
    clientToken = j['clientToken'] as String?;
    clientDeviceLabel = j['clientDeviceLabel'] as String?;
    clientAccount = j['clientAccount'] as String?;
    hubSignInUrl =
        (j['hubSignInUrl'] as String?) ??
        (clientDeviceLabel != null ? clientUrl : null) ??
        '';
    prefetchAttachments = (j['prefetchAttachments'] as bool?) ?? false;
    trustedCertificates = TrustedCertificates.fromJson(j['trustedCertificates']);
    if (j['reminders'] case final Map r) {
      final lead = r['leadMinutes'];
      final morning = r['morning'];
      reminders = ReminderSettings(
        enabled: r['enabled'] is bool ? r['enabled'] as bool : true,
        leadMinutes: lead is int && const [0, 5, 10, 30].contains(lead)
            ? lead
            : 10,
        morning:
            morning is int &&
                morning >= 300 &&
                morning <= 720 &&
                morning % 30 == 0
            ? morning
            : 480,
        permissionAsked: r['permissionAsked'] is bool
            ? r['permissionAsked'] as bool
            : false,
      );
    }
  }

  Map<String, dynamic> toJson() => {
    'theme': theme.name,
    'vaultPath': ?vaultPath,
    'hubEnabled': ?_hubEnabled,
    'scheduleRunsHere': scheduleRunsHere,
    'hubUsesThisComputer': hubUsesThisComputer,
    'shakeToSend': shakeToSend,
    'updatesIncludeNightly': updatesIncludeNightly,
    'updatesAutoDownload': updatesAutoDownload,
    'hubAddress': hubAddress,
    'clientUrl': ?clientUrl,
    'clientToken': ?clientToken,
    'clientDeviceLabel': ?clientDeviceLabel,
    'clientAccount': ?clientAccount,
    'hubSignInUrl': hubSignInUrl,
    'prefetchAttachments': prefetchAttachments,
    if (trustedCertificates.isNotEmpty) 'trustedCertificates': trustedCertificates.toJson(),
    'reminders': {
      'enabled': reminders.enabled,
      'leadMinutes': reminders.leadMinutes,
      'morning': reminders.morning,
      'permissionAsked': reminders.permissionAsked,
    },
  };

  /// Applies [change], saves and notifies.
  void update(void Function(AppSettings s) change) {
    change(this);
    _save();
    notifyListeners();
  }

  set hubEnabled(bool v) => _hubEnabled = v;

  void _save() {
    final f = file;
    if (f == null) return;
    try {
      f.parent.createSync(recursive: true);
      final tmp = File('${f.path}.tmp');
      tmp.writeAsStringSync(
        const JsonEncoder.withIndent('  ').convert(toJson()),
        flush: true,
      );
      tmp.renameSync(f.path);
    } catch (_) {}
  }
}

class SettingsScope extends InheritedNotifier<AppSettings> {
  const SettingsScope({
    super.key,
    required AppSettings settings,
    required super.child,
  }) : super(notifier: settings);
}
