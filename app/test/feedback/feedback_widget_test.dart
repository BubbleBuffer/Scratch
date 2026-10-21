// The "Send feedback" flow on the real core: Ctrl+Shift+F, the Settings row,
// the sidebar footer button, the page "•••" menu, the top-bar button with the
// sidebar collapsed and the mobile Pages screen open the sheet, Send writes one report page under the hidden Feedback page,
// Cancel writes nothing, and the report stays out of the sidebar and search.
//
// The package facts are faked here. Tests run on the Linux machine, so the device facts
// come from the real Linux plugin, registered as the app does at startup: the
// method-channel default cannot describe a Linux device (its data is typed as
// the base class, which the collector casts to LinuxDeviceInfo).

import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/feedback/feedback_sheet.dart';
import 'package:notes_app/ui/icon_x.dart';
import 'package:notes_app/ui/ui.dart';
import 'package:package_info_plus/package_info_plus.dart';

import '../harness.dart';

const _sentWords = 'The sidebar froze zebrafeedback7';

/// Fixed package facts; the device facts come from the Linux plugin.
void _fakePlatformInfo() {
  PackageInfo.setMockInitialValues(
    appName: 'Notes',
    packageName: 'notes.test',
    version: '0.1.0',
    buildNumber: '1',
    buildSignature: '',
  );
  DeviceInfoPlusLinuxPlugin.registerWith();
}

Finder get _sheetText => find.text('What happened?');

Finder _key(String k) => find.byKey(ValueKey(k));

/// Ctrl+Shift+F, the desktop shortcut for the feedback sheet.
/// The feedback icon (not the smile, which stays on "Add icon") in [button].
void _expectFeedbackIcon(Finder button) {
  bool icon(Widget w, Ix ix) => w is IconX && w.icon == ix;
  expect(find.descendant(of: button, matching: find.byWidgetPredicate((w) => icon(w, Ix.feedback))), findsOneWidget);
  expect(find.descendant(of: button, matching: find.byWidgetPredicate((w) => icon(w, Ix.smile))), findsNothing);
}

Future<void> _pressShortcut(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyF);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
}

/// Waits until the sheet is up: the screenshot and diagnostics are async.
Future<void> _waitForSheet(WidgetTester tester) async {
  await settleUntil(tester, () => _sheetText.evaluate().isNotEmpty);
  expect(_sheetText, findsOneWidget);
}

Future<void> _typeReport(WidgetTester tester, String text) async {
  await tester.enterText(find.byKey(const ValueKey('feedback-text')), text);
  await settle(tester);
}

Future<String> _feedbackId(WidgetTester tester) =>
    core(tester, (v) => v.run((n) => n.feedbackPage(), writes: true));

void main() {
  setUpAll(initCore);

  appTest('Ctrl+Shift+F opens the sheet with the screenshot attached', (
    tester,
  ) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: false);
    await _pressShortcut(tester);
    await _waitForSheet(tester);

    expect(find.byKey(const ValueKey('feedback-screenshot')), findsOneWidget);

    // The diagnostics show what will be sent, and the redaction holds.
    await tester.tap(find.byKey(const ValueKey('feedback-diagnostics-toggle')));
    await settle(tester);
    expect(find.textContaining('app version: 0.1.0'), findsOneWidget);
    expect(find.textContaining('page count:'), findsOneWidget);
    // The setting is the default (system); the brightness is whatever the app draws.
    expect(find.textContaining('theme: system ('), findsOneWidget);
  });

  appTest('Send writes one report page under the Feedback page', (
    tester,
  ) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: false);
    await _pressShortcut(tester);
    await _waitForSheet(tester);
    await _typeReport(tester, _sentWords);

    await tester.tap(find.byKey(const ValueKey('feedback-send')));
    await settleUntil(
      tester,
      () => find.text('Feedback saved — thanks').evaluate().isNotEmpty,
    );
    expect(_sheetText, findsNothing);

    final feedbackId = await _feedbackId(tester);
    final reports = await core(tester, (v) => v.listChildren(feedbackId));
    expect(reports, hasLength(1));

    final blocks = await blocksOf(tester, reports.single.id);
    expect(blocks.map((b) => b.kind).toList(), ['paragraph', 'image', 'code']);
    expect(blocks[0].text, _sentWords);
    expect(blocks[1].target, isNotEmpty);
    expect(blocks[2].target, 'text');
    expect(blocks[2].text, contains('app version'));
  });

  appTest('Cancel writes nothing', (tester) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: false);
    final before = await core(tester, (v) => v.pageCount());

    await _pressShortcut(tester);
    await _waitForSheet(tester);
    await _typeReport(tester, _sentWords);
    await tester.tap(find.byKey(const ValueKey('feedback-cancel')));
    await settle(tester);

    expect(_sheetText, findsNothing);
    expect(await core(tester, (v) => v.pageCount()), before);
  });

  appTest('The report is found by id, not by the sidebar or search', (
    tester,
  ) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: false);
    await _pressShortcut(tester);
    await _waitForSheet(tester);
    await _typeReport(tester, _sentWords);
    await tester.tap(find.byKey(const ValueKey('feedback-send')));
    await settleUntil(
      tester,
      () => find.text('Feedback saved — thanks').evaluate().isNotEmpty,
    );

    final feedbackId = await _feedbackId(tester);
    final reportId = (await core(
      tester,
      (v) => v.listChildren(feedbackId),
    )).single.id;

    final top = await core(tester, (v) => v.listChildren(null));
    final topIds = top.map((p) => p.id).toList();
    expect(topIds, isNot(contains(feedbackId)));
    expect(topIds, isNot(contains(reportId)));

    // A fixture page's title is found, so search works and the miss is real.
    expect(
      await core(tester, (v) => v.search(fixtureTitles.first)),
      isNotEmpty,
    );
    final hits = await core(tester, (v) => v.search('zebrafeedback7'));
    expect(hits.map((h) => h.id), isNot(contains(reportId)));

    final page = await core(tester, (v) => v.loadPage(reportId));
    expect(page.id, reportId);
    expect(find.textContaining('Feedback 2026-10-06'), findsNothing);
  });

  appTest('A second shortcut press does not open a second sheet', (
    tester,
  ) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: false);
    await _pressShortcut(tester);
    await _waitForSheet(tester);

    await _pressShortcut(tester);
    await settle(tester);
    expect(_sheetText, findsOneWidget);
  });

  appTest('On a phone the Settings row opens the bottom sheet', (tester) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: true);
    await openMobileSettings(tester);
    await tapAndSettle(
      tester,
      find.byKey(const ValueKey('settings-nav-preferences')),
    );
    await tapAndSettle(tester, find.byKey(const ValueKey('send-feedback')));
    await _waitForSheet(tester);

    expect(find.byType(SheetSurface), findsOneWidget);
    expect(find.byType(FeedbackSheet), findsOneWidget);
  });

  appTest('The sidebar footer button opens the sheet', (tester) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: false);
    expect(_key('sidebar-feedback'), findsOneWidget);
    expect(_key('sidebar-settings'), findsOneWidget);
    _expectFeedbackIcon(_key('sidebar-feedback'));
    await tester.tap(_key('sidebar-feedback'));
    await _waitForSheet(tester);
  });

  appTest('The page menu ends with Send feedback, which opens the sheet', (
    tester,
  ) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: false);
    await openShowcase(tester);
    await tapAndSettle(tester, _key('page-menu'));
    final item = _key('page-menu-feedback');
    expect(item, findsOneWidget);
    expect(
      find.descendant(of: item, matching: find.text('Ctrl+Shift+F')),
      findsOneWidget,
    );
    _expectFeedbackIcon(item);
    // Last row: nothing in the menu sits below it.
    final bottoms = find
        .byWidgetPredicate((w) {
          final k = w.key;
          return k is ValueKey<String> && k.value.startsWith('page-menu-');
        })
        .evaluate()
        .map((e) => tester.getRect(find.byWidget(e.widget)).bottom);
    expect(tester.getRect(item).bottom, bottoms.reduce((a, b) => a > b ? a : b));
    await tester.tap(item);
    await _waitForSheet(tester);
  });

  appTest('With the sidebar collapsed the top bar button opens the sheet', (
    tester,
  ) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: false);
    expect(_key('topbar-feedback'), findsNothing);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.backslash);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await settle(tester);
    expect(_key('sidebar'), findsNothing);
    expect(_key('topbar-feedback'), findsOneWidget);
    _expectFeedbackIcon(_key('topbar-feedback'));
    await tester.tap(_key('topbar-feedback'));
    await _waitForSheet(tester);
  });

  appTest('On a phone the Pages screen has Send feedback below Settings', (
    tester,
  ) async {
    _fakePlatformInfo();
    await pumpApp(tester, mobile: true);
    await openMobileTree(tester);
    await tester.ensureVisible(_key('pages-feedback'));
    await settle(tester);
    expect(
      tester.getRect(_key('pages-feedback')).top,
      greaterThanOrEqualTo(tester.getRect(_key('pages-settings')).bottom),
    );
    _expectFeedbackIcon(_key('pages-feedback'));
    await tester.tap(_key('pages-feedback'));
    await _waitForSheet(tester);
    expect(find.byType(FeedbackSheet), findsOneWidget);
  });
}
