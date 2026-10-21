import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/vault.dart';

import 'harness.dart' as harness;

void main() {
  tearDown(() {
    Clock.fixedAt = null;
    Debounce.instant = false;
    expect(Debounce.pending, 0);
  });

  test('unset clock follows system time', () {
    final before = DateTime.now();
    final now = clockNow();
    final after = DateTime.now();
    expect(now.isBefore(before), isFalse);
    expect(now.isAfter(after), isFalse);
  });

  testWidgets('fixed clock uses real elapsed time, not pumped time', (tester) async {
    final fixed = DateTime(2026, 10, 6, 11, 20);
    Clock.fixedAt = fixed;
    final before = clockNow();
    await tester.pump(const Duration(days: 1));
    final after = clockNow();
    expect(after.isBefore(before), isFalse);
    expect(after.difference(fixed), lessThan(const Duration(minutes: 1)));
    Clock.fixedAt = DateTime(2000);
    expect(clockNow().year, 2000);
  });

  testWidgets('instant debounce remains scheduled and counted', (tester) async {
    Debounce.instant = true;
    final debounce = Debounce();
    addTearDown(debounce.cancel);
    var calls = 0;
    debounce.run(const Duration(seconds: 10), () => calls++);
    expect(calls, 0);
    expect(debounce.scheduled, isTrue);
    expect(Debounce.pending, 1);
    await tester.pump(Duration.zero);
    expect(calls, 1);
    expect(debounce.scheduled, isFalse);
    expect(Debounce.pending, 0);
  });

  testWidgets('instant debounce replaces and cancels pending work', (tester) async {
    Debounce.instant = true;
    final debounce = Debounce();
    addTearDown(debounce.cancel);
    var calls = 0;
    debounce.run(const Duration(seconds: 10), () => calls += 100);
    debounce.run(const Duration(seconds: 10), () => calls++);
    expect(Debounce.pending, 1);
    await tester.pump(Duration.zero);
    expect(calls, 1);
    debounce.run(Duration.zero, () => calls++);
    debounce.cancel();
    await tester.pump(Duration.zero);
    expect(calls, 1);
  });

  testWidgets('production debounce retains its delay', (tester) async {
    final debounce = Debounce();
    addTearDown(debounce.cancel);
    var calls = 0;
    debounce.run(const Duration(seconds: 1), () => calls++);
    await tester.pump(const Duration(milliseconds: 999));
    expect(calls, 0);
    expect(Debounce.pending, 1);
    await tester.pump(const Duration(milliseconds: 1));
    expect(calls, 1);
  });

  testWidgets('settle keeps pumping pending work beyond its animation cap', (tester) async {
    Vault.inFlight.value = 1;
    // Fake time only: reproduce a native reply arriving after 600 frames.
    final reply = Timer(const Duration(seconds: 31), () => Vault.inFlight.value = 0);
    addTearDown(() {
      reply.cancel();
      Vault.inFlight.value = 0;
    });
    await harness.settle(tester);
    expect(Vault.inFlight.value, 0);
  });
}
