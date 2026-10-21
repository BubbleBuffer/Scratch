import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/today_timers.dart';

void main() {
  test('minute refresh uses seconds and milliseconds up to the next boundary', () {
    expect(minuteRefreshDelay(DateTime.utc(2026, 10, 6, 11, 20)), const Duration(minutes: 1));
    expect(minuteRefreshDelay(DateTime.utc(2026, 10, 6, 11, 20, 42, 125)), const Duration(milliseconds: 17875));
    expect(minuteRefreshDelay(DateTime.utc(2026, 10, 6, 23, 59, 59, 999)), const Duration(milliseconds: 1));
  });

  test('repeated fallback hours cannot produce a negative or immediate minute timer', () {
    final first = DateTime.parse('2026-10-25T02:59:59.999+02:00');
    final repeated = DateTime.parse('2026-10-25T02:59:59.999+01:00');
    expect(repeated.difference(first), const Duration(hours: 1));
    expect(minuteRefreshDelay(first), const Duration(milliseconds: 1));
    expect(minuteRefreshDelay(repeated), const Duration(milliseconds: 1));
  });

  test('Berlin fallback at 02:30 schedules both occurrences against their actual instants', () {
    // Explicit offsets describe Berlin's repeated local hour without relying
    // on the test process time zone; DateTime.parse converts them to UTC.
    final first = DateTime.parse('2026-10-25T02:30:12.345+02:00');
    final repeated = DateTime.parse('2026-10-25T02:30:12.345+01:00');
    final midnight = DateTime.parse('2026-10-26T00:00:00+01:00');
    expect(minuteRefreshDelay(first), const Duration(milliseconds: 47655));
    expect(minuteRefreshDelay(repeated), const Duration(milliseconds: 47655));
    expect(midnightRefreshDelay(first, nextMidnight: midnight),
      const Duration(hours: 22, minutes: 30) - const Duration(seconds: 12, milliseconds: 345));
    expect(midnightRefreshDelay(repeated, nextMidnight: midnight),
      const Duration(hours: 21, minutes: 30) - const Duration(seconds: 12, milliseconds: 345));
  });

  test('midnight refresh uses the next calendar day in the input time zone', () {
    final now = DateTime(2026, 10, 6, 11, 20);
    expect(midnightRefreshDelay(now), DateTime(2026, 10, 7).difference(now));
    expect(midnightRefreshDelay(DateTime.utc(2026, 12, 31, 22)), const Duration(hours: 2));
  });

  test('midnight refresh clamps a close, equal or stale boundary to one minute', () {
    final now = DateTime.utc(2026, 10, 25, 23, 59, 59, 999);
    expect(midnightRefreshDelay(now), const Duration(minutes: 1));
    expect(midnightRefreshDelay(now, nextMidnight: now), const Duration(minutes: 1));
    expect(midnightRefreshDelay(now, nextMidnight: now.subtract(const Duration(hours: 1))), const Duration(minutes: 1));
    expect(midnightRefreshDelay(now, nextMidnight: now.add(const Duration(minutes: 1))), const Duration(minutes: 1));
    expect(midnightRefreshDelay(now, nextMidnight: now.add(const Duration(hours: 25))), const Duration(hours: 25));
  });
}
