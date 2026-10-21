/// Delay to the next displayed minute, independent of ambiguous local hours.
Duration minuteRefreshDelay(DateTime now) => Duration(
      milliseconds: Duration.millisecondsPerMinute - now.second * Duration.millisecondsPerSecond - now.millisecond,
    );

/// Recheck the calendar day at local midnight, with a bounded retry delay if
/// a clock adjustment makes the calculated boundary stale or very close.
Duration midnightRefreshDelay(DateTime now, {DateTime? nextMidnight}) {
  final next = nextMidnight ??
      (now.isUtc ? DateTime.utc(now.year, now.month, now.day + 1) : DateTime(now.year, now.month, now.day + 1));
  final delay = next.difference(now);
  return delay < const Duration(minutes: 1) ? const Duration(minutes: 1) : delay;
}
