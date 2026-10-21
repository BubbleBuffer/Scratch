// The schedule format and its pure rules: the round trip keeps keys this
// client does not know, only edited top-level keys are written, the rule text
// and the fake hash, input checks, the synced status and the fake's run times.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/schedule/fake.dart';
import 'package:notes_app/addons/schedule/format.dart';

import 'schedule_fixture.dart';

void main() {
  group('round trip', () {
    test('keeps unknown keys at every level and writes them back', () {
      final json = weeklyDefJson()
        ..['future_key'] = {'a': 1, 'b': ['x']};
      (json['trigger'] as Map<String, Object?>)['future_trigger'] = 'kept';
      ((json['steps'] as List).first as Map<String, Object?>)['future_step'] =
          true;

      final def = ScheduleDef.fromJson(json, tz: 'UTC');

      expect(def.toJson(), equals(json));
    });

    test('an edit writes only the top-level keys it changed', () {
      final before = weeklyDefJson();
      final edited = ScheduleDef.fromJson(before, tz: 'UTC').copyWith(
        runner: 'desk-2',
      );

      final changes = changedKeys(before, edited.toJson());

      expect(changes.keys, ['runner']);
      expect(changes['runner'], 'desk-2');
    });

    test('a removed optional key shows as a removal (null value)', () {
      final before = weeklyDefJson();
      final after = ScheduleDef.fromJson(before, tz: 'UTC').copyWith(
        runner: null,
      );

      final changes = changedKeys(before, after.toJson());

      expect(changes.containsKey('runner'), isTrue);
      expect(changes['runner'], isNull);
    });
  });

  group('rule text', () {
    test('a weekly rule with two weekdays', () {
      final rule = ScheduleRule.parse('FREQ=WEEKLY;BYDAY=MO')!.withByDay([
        'MO',
        'FR',
      ]);
      expect(rule.toText(), 'FREQ=WEEKLY;BYDAY=MO,FR');
    });

    test('switching a daily rule to weekly defaults to the trigger day', () {
      final rule = ScheduleRule.parse('FREQ=DAILY')!.withFreq(
        ScheduleFreq.weekly,
        defaultDay: 'MO',
        defaultMonthDay: 5,
      );
      expect(rule.toText(), 'FREQ=WEEKLY;BYDAY=MO');
    });

    test('switching to monthly keeps the day of month', () {
      final rule = ScheduleRule.parse('FREQ=DAILY')!.withFreq(
        ScheduleFreq.monthly,
        defaultDay: 'MO',
        defaultMonthDay: 5,
      );
      expect(rule.toText(), 'FREQ=MONTHLY;BYMONTHDAY=5');
    });
  });

  group('fake hash', () {
    // The engine hashes runner, tools and steps. The fake stands in for it,
    // so the view can be tested against a hash that moves the same way.
    test('ignores the trigger and changes with the runner', () {
      final base = ScheduleDef.fromJson(weeklyDefJson(), tz: 'UTC');
      final retimed = base.copyWith(
        trigger: base.trigger.copyWith(at: '2026-10-05T09:30'),
      );
      final moved = base.copyWith(runner: 'desk-2');

      expect(fakeHash(retimed), fakeHash(base));
      expect(fakeHash(moved), isNot(fakeHash(base)));
    });
  });

  group('input checks', () {
    test('a 24-hour time is accepted, other forms are named', () {
      expect(validateTime('08:00'), isNull);
      expect(validateTime('8am'), 'Use 24-hour time, for example 08:00.');
      expect(validateTime('25:00'), 'Use a time between 00:00 and 23:59.');
    });
  });

  group('status', () {
    test('reads the runner state and the last run from the synced status', () {
      final status = ScheduleStatus.fromJson(statusJson());

      expect(status.runnerState, ScheduleHealth.ok);
      expect(status.nextRun, DateTime.parse('2026-10-12T08:00'));
      expect(status.lastRun?.result, 'created');
      expect(status.lastRun?.page, 'pg-42');
    });

    test('an unknown runner state reads as no state', () {
      final status = ScheduleStatus.fromJson({'runner_state': 'later'});

      expect(status.runnerState, isNull);
    });
  });

  group('next runs (fake engine)', () {
    test('a Monday rule gives the next three Mondays at 08:00', () {
      final trigger = ScheduleDef.fromJson(weeklyDefJson(), tz: 'UTC').trigger;

      final runs = nextRunsOf(trigger, scheduleNow);

      expect(runs, [
        DateTime(2026, 10, 12, 8),
        DateTime(2026, 10, 19, 8),
        DateTime(2026, 10, 26, 8),
      ]);
    });
  });
}
