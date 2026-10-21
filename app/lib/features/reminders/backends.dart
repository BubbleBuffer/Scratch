import 'dart:async';
import 'dart:io';

import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:timezone/timezone.dart' as tz;

import '../../core/clock.dart';
import '../../ui/app_name.dart';
import '../settings/settings_store.dart';
import 'reminder_plan.dart';

abstract class ReminderBackend {
  Future<void> init();
  Future<bool> ensurePermission();
  Future<void> apply(List<PlannedReminder> plan);
  Future<void> cancelAll();
  Stream<String> get taps;
  Future<void> dispose() async {}
}

ReminderBackend createReminderBackend(
  AppSettings settings, {
  required void Function(String title, String body) toast,
}) => Platform.isAndroid
    ? AndroidReminderBackend(settings: settings)
    : DesktopReminderBackend(toast: toast);

NotificationDetails _details(PlannedReminder reminder) {
  final summary = reminder.kind == ReminderKind.summary;
  return NotificationDetails(
    android: AndroidNotificationDetails(
      summary ? 'summary' : 'reminders',
      summary ? 'Morning summary' : 'Reminders',
      importance: summary ? Importance.defaultImportance : Importance.high,
      priority: summary ? Priority.defaultPriority : Priority.high,
    ),
    macOS: const DarwinNotificationDetails(),
    linux: const LinuxNotificationDetails(),
    windows: const WindowsNotificationDetails(),
  );
}

class AndroidReminderBackend extends ReminderBackend {
  AndroidReminderBackend({
    required this.settings,
    FlutterLocalNotificationsPlugin? plugin,
  }) : _plugin = plugin ?? FlutterLocalNotificationsPlugin();

  final AppSettings settings;
  final FlutterLocalNotificationsPlugin _plugin;
  final _taps = StreamController<String>.broadcast();
  final Map<int, PlannedReminder> _scheduled = {};
  final Set<int> _pendingIds = {};

  @override
  Stream<String> get taps => _taps.stream;

  @override
  Future<void> init() async {
    await _plugin.initialize(
      settings: const InitializationSettings(
        android: AndroidInitializationSettings('ic_notification'),
      ),
      onDidReceiveNotificationResponse: (response) {
        final payload = response.payload;
        if (payload != null && !_taps.isClosed) _taps.add(payload);
      },
    );
    final android = _plugin
        .resolvePlatformSpecificImplementation<
          AndroidFlutterLocalNotificationsPlugin
        >();
    await android?.createNotificationChannel(
      const AndroidNotificationChannel(
        'reminders',
        'Reminders',
        importance: Importance.high,
      ),
    );
    await android?.createNotificationChannel(
      const AndroidNotificationChannel(
        'summary',
        'Morning summary',
        importance: Importance.defaultImportance,
      ),
    );
    _pendingIds.addAll(
      (await _plugin.pendingNotificationRequests()).map((r) => r.id),
    );
    final launch = await _plugin.getNotificationAppLaunchDetails();
    final payload = launch?.notificationResponse?.payload;
    if (launch?.didNotificationLaunchApp == true &&
        payload != null &&
        !_taps.isClosed) {
      _taps.add(payload);
    }
  }

  @override
  Future<bool> ensurePermission() async {
    final android = _plugin
        .resolvePlatformSpecificImplementation<
          AndroidFlutterLocalNotificationsPlugin
        >();
    if (!settings.reminders.permissionAsked) {
      // Persist before awaiting the OS prompt so concurrent replans cannot ask twice.
      settings.update(
        (s) => s.reminders = s.reminders.copyWith(permissionAsked: true),
      );
      return await android?.requestNotificationsPermission() ?? false;
    }
    return await android?.areNotificationsEnabled() ?? false;
  }

  @override
  Future<void> apply(List<PlannedReminder> plan) async {
    final ids = plan.map((r) => r.id).toSet();
    for (final id in {..._pendingIds, ..._scheduled.keys}.difference(ids)) {
      await _plugin.cancel(id: id);
      _scheduled.remove(id);
      _pendingIds.remove(id);
    }
    for (final reminder in plan) {
      if (_scheduled[reminder.id] == reminder) continue;
      await _plugin.zonedSchedule(
        id: reminder.id,
        title: reminder.title,
        body: reminder.body,
        scheduledDate: tz.TZDateTime.from(reminder.instant, tz.UTC),
        notificationDetails: _details(reminder),
        androidScheduleMode: AndroidScheduleMode.inexactAllowWhileIdle,
        payload: reminder.payload,
      );
      _scheduled[reminder.id] = reminder;
      _pendingIds.add(reminder.id);
    }
  }

  @override
  Future<void> cancelAll() async {
    await _plugin.cancelAll();
    _scheduled.clear();
    _pendingIds.clear();
  }

  @override
  Future<void> dispose() => _taps.close();
}

/// One pure desktop clock step. Expired reminders are consumed as well, so a
/// later clock adjustment cannot make a dropped reminder appear.
class ReminderDueResult {
  const ReminderDueResult({required this.reminders, required this.fired});
  final List<PlannedReminder> reminders;
  final Set<int> fired;
}

ReminderDueResult due(
  List<PlannedReminder> plan,
  Set<int> fired,
  DateTime now,
) {
  final consumed = {...fired};
  final reminders = <PlannedReminder>[];
  for (final reminder in plan) {
    if (consumed.contains(reminder.id) || reminder.instant.isAfter(now)) {
      continue;
    }
    consumed.add(reminder.id);
    if (now.difference(reminder.instant) <= const Duration(minutes: 30)) {
      reminders.add(reminder);
    }
  }
  return ReminderDueResult(
    reminders: List.unmodifiable(reminders),
    fired: Set.unmodifiable(consumed),
  );
}

class DesktopReminderBackend extends ReminderBackend {
  DesktopReminderBackend({
    required this.toast,
    FlutterLocalNotificationsPlugin? plugin,
    DateTime Function()? now,
    this.startTicker = true,
  }) : _plugin = plugin ?? FlutterLocalNotificationsPlugin(),
       _now = now ?? clockNow;

  final void Function(String title, String body) toast;
  final FlutterLocalNotificationsPlugin _plugin;
  final DateTime Function() _now;
  final bool startTicker;
  final _taps = StreamController<String>.broadcast();
  Timer? _timer;
  bool _fallback = false;
  List<PlannedReminder> _plan = [];
  Set<int> _fired = {};

  @override
  Stream<String> get taps => _taps.stream;

  @override
  Future<void> init() async {
    try {
      await _plugin.initialize(
        settings: const InitializationSettings(
          macOS: DarwinInitializationSettings(),
          linux: LinuxInitializationSettings(defaultActionName: 'Open'),
          windows: WindowsInitializationSettings(
            appName: appName,
            appUserModelId: 'BubbleBuffer.Notes',
            guid: 'b8e41f46-acee-4413-83a0-92dd51ce3d8e',
          ),
        ),
        onDidReceiveNotificationResponse: (response) {
          final payload = response.payload;
          if (payload != null && !_taps.isClosed) _taps.add(payload);
        },
      );
    } catch (_) {
      _fallback = true;
    }
    if (startTicker) {
      _timer = Timer.periodic(
        const Duration(minutes: 1),
        (_) => unawaited(tick()),
      );
    }
  }

  @override
  Future<bool> ensurePermission() async => true;

  @override
  Future<void> apply(List<PlannedReminder> plan) async {
    _plan = List.of(plan);
  }

  /// Exposed for deterministic ticker tests, without launching a native app.
  Future<void> tick() async {
    final result = due(_plan, _fired, _now());
    _fired = result.fired;
    for (final reminder in result.reminders) {
      if (_fallback) {
        toast(reminder.title, reminder.body);
      } else {
        await _plugin.show(
          id: reminder.id,
          title: reminder.title,
          body: reminder.body,
          notificationDetails: _details(reminder),
          payload: reminder.payload,
        );
      }
    }
  }

  @override
  Future<void> cancelAll() async {
    _plan = [];
    // Keep consumed ids for this session: toggling reminders cannot fire twice.
  }

  @override
  Future<void> dispose() async {
    _timer?.cancel();
    _plan = [];
    await _taps.close();
  }
}

class FakeReminderBackend extends ReminderBackend {
  FakeReminderBackend({this.settings, this.permissionGranted = true});
  final AppSettings? settings;
  bool permissionGranted;
  int initCalls = 0;
  int permissionCalls = 0;
  int permissionRequests = 0;
  int cancelCalls = 0;
  bool _permissionAsked = false;
  final List<List<PlannedReminder>> appliedPlans = [];
  final _taps = StreamController<String>.broadcast();

  @override
  Stream<String> get taps => _taps.stream;
  void emitTap(String payload) => _taps.add(payload);

  @override
  Future<void> init() async {
    initCalls++;
  }

  @override
  Future<bool> ensurePermission() async {
    permissionCalls++;
    if (!(settings?.reminders.permissionAsked ?? _permissionAsked)) {
      permissionRequests++;
      _permissionAsked = true;
      settings?.update(
        (s) => s.reminders = s.reminders.copyWith(permissionAsked: true),
      );
    }
    return permissionGranted;
  }

  @override
  Future<void> apply(List<PlannedReminder> plan) async =>
      appliedPlans.add(List.of(plan));
  @override
  Future<void> cancelAll() async {
    cancelCalls++;
  }

  @override
  Future<void> dispose() => _taps.close();
}
