// "Run schedules on this device": the saved flag changes only after the runner
// has changed. A failure keeps the old flag and exposes the error on the
// switch's state; the next successful change clears it.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/schedule/settings.dart';
import 'package:notes_app/services/addon_services.dart';

void main() {
  late Directory dir;
  late File file;
  late AppSettings settings;
  final calls = <bool>[];
  String? Function()? failure;
  Future<String?> Function(bool on)? runnerOverride;

  Future<String?> runner(bool on) async {
    calls.add(on);
    final override = runnerOverride;
    if (override != null) return override(on);
    return failure?.call();
  }

  ScheduleRunnerSwitch switchWith(Future<String?> Function(bool on) run) =>
      ScheduleRunnerSwitch(settings: settings, runner: run);

  bool savedFlag() =>
      jsonDecode(file.readAsStringSync())['scheduleRunsHere'] as bool;

  setUp(() {
    dir = Directory.systemTemp.createTempSync('notes-runner-switch-');
    file = File('${dir.path}/settings.json');
    settings = AppSettings(file: file);
    calls.clear();
    failure = null;
    runnerOverride = null;
  });

  tearDown(() {
    settings.dispose();
    dir.deleteSync(recursive: true);
  });

  test('success turns the runner on and saves the flag', () async {
    final switcher = switchWith(runner);
    expect(switcher.on, isFalse);

    expect(await switcher.set(true), isTrue);

    expect(calls, [true]);
    expect(switcher.on, isTrue);
    expect(switcher.error, isNull);
    expect(settings.scheduleRunsHere, isTrue);
    expect(savedFlag(), isTrue);
    switcher.dispose();
  });

  test('a failed start keeps the flag off and exposes the error', () async {
    failure = () => 'runner did not start';
    final switcher = switchWith(runner);

    expect(await switcher.set(true), isFalse);

    expect(calls, [true]);
    expect(switcher.on, isFalse);
    expect(settings.scheduleRunsHere, isFalse);
    expect(switcher.error, 'runner did not start');
    expect(file.existsSync(), isFalse, reason: 'nothing was saved');
    switcher.dispose();
  });

  test('a failed stop keeps the old flag on and exposes the error', () async {
    final switcher = switchWith(runner);
    expect(await switcher.set(true), isTrue);

    failure = () => 'runner is busy';
    expect(await switcher.set(false), isFalse);

    expect(calls, [true, false]);
    expect(switcher.on, isTrue);
    expect(savedFlag(), isTrue);
    expect(switcher.error, 'runner is busy');
    switcher.dispose();
  });

  test('a throw from the runner counts as a failure', () async {
    final switcher = switchWith((on) => Future.error(StateError('no vault')));

    expect(await switcher.set(true), isFalse);

    expect(switcher.on, isFalse);
    expect(settings.scheduleRunsHere, isFalse);
    expect(switcher.error, contains('no vault'));
    switcher.dispose();
  });

  test('the error clears on the next successful change', () async {
    failure = () => 'first try failed';
    final switcher = switchWith(runner);
    expect(await switcher.set(true), isFalse);
    expect(switcher.error, 'first try failed');

    failure = null;
    expect(await switcher.set(true), isTrue);

    expect(switcher.error, isNull);
    expect(switcher.on, isTrue);
    expect(savedFlag(), isTrue);
    switcher.dispose();
  });

  test('listeners hear the pending state, then the result', () async {
    final gate = Completer<String?>();
    runnerOverride = (on) => gate.future;
    final switcher = switchWith(runner);
    final seen = <String>[];
    switcher.addListener(() {
      seen.add(
        'busy=${switcher.busy} on=${switcher.on} error=${switcher.error}',
      );
    });

    final pending = switcher.set(true);
    expect(switcher.busy, isTrue);
    expect(
      settings.scheduleRunsHere,
      isFalse,
      reason: 'not saved while pending',
    );

    gate.complete('denied');
    expect(await pending, isFalse);

    expect(seen, [
      'busy=true on=false error=null',
      'busy=false on=false error=denied',
    ]);
    switcher.dispose();
  });

  test('a change while another is pending is ignored', () async {
    final gate = Completer<String?>();
    runnerOverride = (on) => gate.future;
    final switcher = switchWith(runner);

    final first = switcher.set(true);
    expect(await switcher.set(false), isFalse);
    expect(calls, [true], reason: 'the runner was called once');

    gate.complete(null);
    expect(await first, isTrue);
    expect(switcher.on, isTrue);
    switcher.dispose();
  });

  test(
    'disposed while the runner is pending: the flag saves, nothing notifies',
    () async {
      final gate = Completer<String?>();
      runnerOverride = (on) => gate.future;
      final switcher = switchWith(runner);
      var notified = 0;
      switcher.addListener(() => notified++);

      final pending = switcher.set(true);
      final beforeDispose = notified;
      switcher.dispose();

      gate.complete(null);
      expect(await pending, isTrue);
      expect(notified, beforeDispose, reason: 'no notification after dispose');
      expect(settings.scheduleRunsHere, isTrue);
      expect(savedFlag(), isTrue);
    },
  );
}
