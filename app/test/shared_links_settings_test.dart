// Settings › Shared links on its own: the rows with their titles and "Deleted
// page", Copy, Open note (disabled for a deleted page), Revoke with the inline
// confirm and Cancel, the empty line, and the section listed only with a hub
// sign-in. The body is pumped directly with the app's theme and fonts and no
// bridge; the hub is an in-memory fake client (`SharePopup.debugClientFor`).

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/features/settings/settings_screen.dart';
import 'package:notes_app/features/share/share_client.dart';
import 'package:notes_app/features/share/share_popup.dart';
import 'package:notes_app/features/share/shared_links_settings.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';

Finder key(String k) => find.byKey(ValueKey(k));

const _day = 24 * 60 * 60 * 1000;

class _FakeShares extends ShareClient {
  _FakeShares()
    : super(hubUrl: 'https://hub.example.com', token: 'device', deviceId: 'dev');

  final shares = <Share>[];
  final revoked = <String>[];

  @override
  Future<List<Share>> listAll() async => List.of(shares);

  @override
  Future<List<Share>> list(String pageId) async =>
      [for (final s in shares) if (s.page == pageId) s];

  @override
  Future<void> revoke(String id) async {
    revoked.add(id);
    shares.removeWhere((s) => s.id == id);
  }
}

/// Records what the app puts on the clipboard.
List<String> recordClipboard(WidgetTester t) {
  final copied = <String>[];
  t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
    SystemChannels.platform,
    (call) async {
      if (call.method == 'Clipboard.setData') {
        copied.add((call.arguments as Map)['text'] as String);
      }
      return null;
    },
  );
  addTearDown(
    () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      null,
    ),
  );
  return copied;
}

/// Pumps the section in the settings panel's right-hand pane, with the app's
/// theme, fonts and fixed clock. No bridge.
Future<void> pumpSection(WidgetTester t) async {
  await t.runAsync(loadFonts);
  sansFamily = 'Atkinson Hyperlegible Next';
  monoFamily = 'Fragment Mono';
  Clock.fixedAt = DateTime(2026, 10, 23, 12);
  t.view.devicePixelRatio = 1.0;
  t.view.physicalSize = const Size(1440, 900);
  addTearDown(() {
    t.view.reset();
    Clock.fixedAt = null;
  });

  final colors = AppColors.light;
  await t.pumpWidget(
    MediaQuery(
      data: MediaQueryData.fromView(t.view),
      child: Directionality(
        textDirection: TextDirection.ltr,
        child: AppTheme(
          colors: colors,
          formFactor: FormFactor.desktop,
          child: DefaultTextStyle(
            style: ts(TypeScale.desktop.body, colors.textPrimary),
            child: ColoredBox(
              color: colors.bgPrimary,
              child: const SingleChildScrollView(
                padding: EdgeInsets.fromLTRB(60, 36, 60, 48),
                child: SharedLinksBody(),
              ),
            ),
          ),
        ),
      ),
    ),
  );
  await settle(t);
}

void main() {
  late _FakeShares hub;
  final now = DateTime(2026, 10, 23, 12).millisecondsSinceEpoch;

  setUp(() {
    hub = _FakeShares()
      ..shares.addAll([
        Share(
          id: 'a1',
          url: 'https://hub.example.com/s/a1',
          page: 'page-garden',
          visibility: 'public',
          kind: 'live',
          created: now - _day,
          title: 'Garden plan',
        ),
        Share(
          id: 'a2',
          url: 'https://hub.example.com/s/a2',
          page: 'page-old',
          visibility: 'private',
          kind: 'snapshot',
          created: now - 3 * _day,
        ),
      ]);
    SharePopup.debugClientFor = (_) => hub;
  });
  tearDown(() => SharePopup.debugClientFor = null);

  testWidgets('lists each link with its page title, kind and date', (t) async {
    await pumpSection(t);

    expect(find.text('Shared links'), findsOneWidget);
    expect(key('share-row-a1'), findsOneWidget);
    expect(key('share-row-a2'), findsOneWidget);
    expect(find.text('Garden plan'), findsOneWidget);
    expect(find.text('Deleted page'), findsOneWidget);
    expect(find.text('Public · Live link'), findsOneWidget);
    expect(find.text('Private · This version'), findsOneWidget);
    expect(find.text('Yesterday'), findsOneWidget);
    expect(find.text('3d ago'), findsOneWidget);
  });

  testWidgets('Open note is disabled for a deleted page only', (t) async {
    await pumpSection(t);

    expect(t.widget<TextBtn>(key('share-open-a1')).onTap, isNotNull);
    expect(t.widget<TextBtn>(key('share-open-a2')).onTap, isNull);
  });

  testWidgets('Copy copies the link and reads Copied', (t) async {
    final copied = recordClipboard(t);
    await pumpSection(t);

    await tapAndSettle(t, key('share-copy-a1'));

    expect(copied, ['https://hub.example.com/s/a1']);
    expect(find.text('Copied'), findsOneWidget);
    await t.pump(const Duration(seconds: 3));
    expect(find.text('Copied'), findsNothing);
  });

  testWidgets('Revoke asks first, then calls the hub and removes the row', (t) async {
    await pumpSection(t);

    await tapAndSettle(t, key('share-revoke-a2'));
    expect(
      find.text('Revoke this link? It stops working for everyone.'),
      findsOneWidget,
    );
    expect(hub.revoked, isEmpty);

    await tapAndSettle(t, key('share-revoke-confirm-a2'));

    expect(hub.revoked, ['a2']);
    expect(key('share-row-a2'), findsNothing);
    expect(key('share-row-a1'), findsOneWidget);
  });

  testWidgets('Cancel restores the row without calling the hub', (t) async {
    await pumpSection(t);

    await tapAndSettle(t, key('share-revoke-a1'));
    await tapAndSettle(t, key('share-cancel-a1'));

    expect(hub.revoked, isEmpty);
    expect(key('share-confirm-a1'), findsNothing);
    expect(key('share-row-a1'), findsOneWidget);
    expect(find.text('Garden plan'), findsOneWidget);
  });

  testWidgets('with no links the section says so', (t) async {
    hub.shares.clear();
    await pumpSection(t);

    expect(
      find.text('No shared links yet. Share a note from its menu.'),
      findsOneWidget,
    );
    expect(key('shared-links-empty'), findsOneWidget);
  });

  testWidgets('the section is listed only with a hub sign-in', (t) async {
    late List<SettingsSection> visible;
    Future<void> pumpList() async {
      await t.pumpWidget(
        Directionality(
          textDirection: TextDirection.ltr,
          child: Builder(
            builder: (ctx) {
              visible = SettingsSection.visibleFor(ctx);
              return const SizedBox();
            },
          ),
        ),
      );
    }

    await pumpList();
    expect(visible, contains(SettingsSection.sharedLinks));
    expect(
      visible.indexOf(SettingsSection.sharedLinks),
      visible.indexOf(SettingsSection.storage) + 1,
    );

    SharePopup.debugClientFor = (_) => null;
    await pumpList();
    expect(visible, isNot(contains(SettingsSection.sharedLinks)));
  });
}
