import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/reminders/backends.dart';
import 'package:notes_app/features/reminders/reminder_plan.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:timezone/timezone.dart' as tz;

final now = DateTime(2026, 10, 6, 11, 20);
PlannedReminder reminder(int id, DateTime at, {String title = 'Meeting'}) =>
    PlannedReminder(
      id: id,
      kind: ReminderKind.event,
      instant: at,
      title: title,
      body: '14:00 · in 10 min',
      payload: '{"page":"page","block":null}',
    );

class _AndroidPlugin extends AndroidFlutterLocalNotificationsPlugin {
  int requests = 0;
  int reads = 0;
  bool granted = false;
  final List<AndroidNotificationChannel> channels = [];
  @override
  Future<bool?> requestNotificationsPermission() async {
    requests++;
    return granted;
  }

  @override
  Future<bool?> areNotificationsEnabled() async {
    reads++;
    return granted;
  }

  @override
  Future<void> createNotificationChannel(
    AndroidNotificationChannel channel,
  ) async => channels.add(channel);
}

class _Plugin implements FlutterLocalNotificationsPlugin {
  final android = _AndroidPlugin();
  bool failInit = false;
  InitializationSettings? initialized;
  DidReceiveNotificationResponseCallback? onTap;
  List<PendingNotificationRequest> pending = [];
  NotificationAppLaunchDetails? launch;
  final List<int> canceled = [];
  final List<PlannedReminder> shown = [];
  final List<
    ({
      int id,
      String? title,
      tz.TZDateTime at,
      AndroidScheduleMode mode,
      NotificationDetails details,
    })
  >
  scheduled = [];

  @override
  Future<bool?> initialize({
    required InitializationSettings settings,
    DidReceiveNotificationResponseCallback? onDidReceiveNotificationResponse,
    DidReceiveBackgroundNotificationResponseCallback?
    onDidReceiveBackgroundNotificationResponse,
  }) async {
    if (failInit) throw StateError('No native notification service');
    initialized = settings;
    onTap = onDidReceiveNotificationResponse;
    return true;
  }

  @override
  T? resolvePlatformSpecificImplementation<
    T extends FlutterLocalNotificationsPlatform
  >() => android as T;
  @override
  Future<List<PendingNotificationRequest>>
  pendingNotificationRequests() async => pending;
  @override
  Future<NotificationAppLaunchDetails?>
  getNotificationAppLaunchDetails() async => launch;
  @override
  Future<void> cancel({required int id, String? tag}) async => canceled.add(id);
  @override
  Future<void> cancelAll() async {
    canceled.addAll(pending.map((r) => r.id));
  }

  @override
  Future<void> zonedSchedule({
    required int id,
    required tz.TZDateTime scheduledDate,
    required NotificationDetails notificationDetails,
    required AndroidScheduleMode androidScheduleMode,
    String? title,
    String? body,
    String? payload,
    DateTimeComponents? matchDateTimeComponents,
  }) async {
    scheduled.add((
      id: id,
      title: title,
      at: scheduledDate,
      mode: androidScheduleMode,
      details: notificationDetails,
    ));
  }

  @override
  Future<void> show({
    required int id,
    String? title,
    String? body,
    NotificationDetails? notificationDetails,
    String? payload,
  }) async {
    shown.add(
      PlannedReminder(
        id: id,
        kind: ReminderKind.event,
        instant: now,
        title: title ?? '',
        body: body ?? '',
        payload: payload ?? '',
      ),
    );
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  test('desktop step fires only passed reminders and preserves its inputs', () {
    final passed = reminder(1, now.subtract(const Duration(minutes: 1)));
    final future = reminder(2, now.add(const Duration(seconds: 1)));
    final fired = <int>{};
    final result = due([passed, future], fired, now);
    expect(result.reminders, [passed]);
    expect(result.fired, {1});
    expect(fired, isEmpty);
    expect(due([passed, future], result.fired, now).reminders, isEmpty);
  });

  test(
    'desktop step drops more than 30 minutes late, including after sleep',
    () {
      final boundary = reminder(1, now.subtract(const Duration(minutes: 30)));
      final late = reminder(
        2,
        now.subtract(const Duration(minutes: 30, milliseconds: 1)),
      );
      final result = due([boundary, late], {}, now);
      expect(result.reminders, [boundary]);
      expect(result.fired, {1, 2});
      expect(
        due(
          [boundary, late],
          result.fired,
          now.subtract(const Duration(minutes: 1)),
        ).reminders,
        isEmpty,
      );
    },
  );

  test(
    'desktop shows native reminders with the fixed Windows identity',
    () async {
      final plugin = _Plugin();
      final backend = DesktopReminderBackend(
        plugin: plugin,
        toast: (_, _) => fail('Unexpected toast'),
        now: () => now,
        startTicker: false,
      );
      await backend.init();
      expect(plugin.initialized!.windows!.appName, 'Scratch');
      expect(plugin.initialized!.windows!.appUserModelId, 'BubbleBuffer.Notes');
      await backend.apply([reminder(1, now)]);
      await backend.tick();
      await backend.tick();
      expect(plugin.shown.map((r) => r.id), [1]);
      await backend.cancelAll();
      await backend.tick();
      expect(plugin.shown, hasLength(1));
      await backend.dispose();
    },
  );

  test(
    'desktop init failure uses in-app toast for the same reminders',
    () async {
      final plugin = _Plugin()..failInit = true;
      final toasts = <String>[];
      final backend = DesktopReminderBackend(
        plugin: plugin,
        toast: (title, body) => toasts.add('$title\n$body'),
        now: () => now,
        startTicker: false,
      );
      await backend.init();
      final item = reminder(1, now);
      await backend.apply([item]);
      await backend.tick();
      await backend.tick();
      expect(toasts, ['${item.title}\n${item.body}']);
      expect(plugin.shown, isEmpty);
      await backend.dispose();
    },
  );

  test(
    'Android asks once and reads permission on subsequent checks and boots',
    () async {
      final settings = AppSettings();
      final plugin = _Plugin();
      final backend = AndroidReminderBackend(
        settings: settings,
        plugin: plugin,
      );
      await backend.init();
      expect(plugin.android.requests, 0);
      expect(await backend.ensurePermission(), false);
      expect(settings.reminders.permissionAsked, true);
      expect(await backend.ensurePermission(), false);
      plugin.android.granted = true;
      final nextBoot = AndroidReminderBackend(
        settings: settings,
        plugin: plugin,
      );
      expect(await nextBoot.ensurePermission(), true);
      expect(plugin.android.requests, 1);
      expect(plugin.android.reads, 2);
      await backend.dispose();
      await nextBoot.dispose();
    },
  );

  test('Android reconciles boot pending ids and schedules changed reminders in UTC', () async {
    final plugin = _Plugin()
      ..pending = [const PendingNotificationRequest(99, 'Old', 'Old', 'today')];
    final backend = AndroidReminderBackend(
      settings: AppSettings(),
      plugin: plugin,
    );
    await backend.init();
    expect(plugin.android.channels.map((c) => c.id), [
      'reminders',
      'agent-runs',
      'summary',
    ]);
    final item = reminder(1, now.add(const Duration(minutes: 5)));
    await backend.apply([item]);
    expect(plugin.canceled, [99]);
    expect(plugin.scheduled.single.at.location, tz.UTC);
    expect(
      plugin.scheduled.single.at.millisecondsSinceEpoch,
      item.instant.millisecondsSinceEpoch,
    );
    expect(
      plugin.scheduled.single.mode,
      AndroidScheduleMode.inexactAllowWhileIdle,
    );
    await backend.apply([item]);
    expect(plugin.scheduled, hasLength(1));
    await backend.apply([reminder(1, item.instant, title: 'Renamed')]);
    expect(plugin.scheduled, hasLength(2));
    await backend.apply([]);
    expect(plugin.canceled, [99, 1]);
    await backend.dispose();
  });

  test('Android emits launch and subsequent notification taps', () async {
    final plugin = _Plugin()
      ..launch = const NotificationAppLaunchDetails(
        true,
        notificationResponse: NotificationResponse(
          notificationResponseType:
              NotificationResponseType.selectedNotification,
          payload: 'today',
        ),
      );
    final backend = AndroidReminderBackend(
      settings: AppSettings(),
      plugin: plugin,
    );
    final taps = <String>[];
    final subscription = backend.taps.listen(taps.add);
    await backend.init();
    plugin.onTap!(
      const NotificationResponse(
        notificationResponseType: NotificationResponseType.selectedNotification,
        payload: '{"page":"page"}',
      ),
    );
    await Future<void>.delayed(Duration.zero);
    expect(taps, ['today', '{"page":"page"}']);
    await subscription.cancel();
    await backend.dispose();
  });
}
