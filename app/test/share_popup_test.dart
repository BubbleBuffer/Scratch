// The Share popup on its own: the two choices with their defaults, a link
// created with the chosen values (copied, shown, listed), the list with kind
// and date, Copy, Revoke, an error line when a call fails, and the signed-out
// note. The popup is pumped directly, with the app's theme and fonts and no
// bridge; the hub is an in-memory fake client (`SharePopup.debugClientFor`).
// The page-menu entry and the full app are covered by share_menu_test.dart.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/features/share/share_client.dart';
import 'package:notes_app/features/share/share_popup.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';

Finder key(String k) => find.byKey(ValueKey(k));

const _day = 24 * 60 * 60 * 1000;
const _page = 'page-garden';

class _FakeShares extends ShareClient {
  _FakeShares()
    : super(hubUrl: 'https://hub.example.com', token: 'device', deviceId: 'dev');

  final shares = <Share>[];
  final created = <(String, String, String)>[];
  final revoked = <String>[];

  /// When set, the next create fails with this message.
  String? createError;
  int _n = 0;

  @override
  Future<List<Share>> list(String pageId) async =>
      [for (final s in shares) if (s.page == pageId) s];

  @override
  Future<Share> create(
    String pageId, {
    required String visibility,
    required String kind,
  }) async {
    final error = createError;
    if (error != null) throw ShareException(error, status: 500);
    created.add((pageId, visibility, kind));
    _n++;
    final share = Share(
      id: 'new$_n',
      url: 'https://hub.example.com/s/new$_n',
      page: pageId,
      visibility: visibility,
      kind: kind,
      created: clockNow().millisecondsSinceEpoch,
    );
    shares.insert(0, share);
    return share;
  }

  @override
  Future<void> revoke(String id) async {
    revoked.add(id);
    shares.removeWhere((s) => s.id == id);
  }
}

/// Pumps the popup in its app presentation (a 480-wide panel on desktop, a
/// sheet on a phone) with the app's theme, fonts and fixed clock. No bridge:
/// the popup needs only the theme, the fonts and an Overlay for its toast.
Future<void> pumpPopup(
  WidgetTester t, {
  bool mobile = false,
  bool dark = false,
}) async {
  await t.runAsync(loadFonts);
  sansFamily = 'Atkinson Hyperlegible Next';
  monoFamily = 'Fragment Mono';
  Clock.fixedAt = DateTime(2026, 10, 23, 12);
  t.view.devicePixelRatio = mobile ? 2.0 : 1.0;
  t.view.physicalSize = mobile ? const Size(750, 1624) : const Size(1440, 900);
  t.platformDispatcher.platformBrightnessTestValue = dark
      ? Brightness.dark
      : Brightness.light;
  addTearDown(() {
    t.view.reset();
    t.platformDispatcher.clearAllTestValues();
    Clock.fixedAt = null;
  });

  final colors = dark ? AppColors.darkTheme : AppColors.light;
  final popup = SharePopup(pageId: _page, pageTitle: 'Garden plan');
  final presented = mobile
      ? Align(
          alignment: Alignment.bottomCenter,
          child: SizedBox(
            width: 375,
            child: SheetSurface(children: [popup]),
          ),
        )
      : Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(
              minWidth: 480,
              maxWidth: 480,
              maxHeight: 640,
            ),
            child: MenuSurface(child: popup),
          ),
        );
  await t.pumpWidget(
    MediaQuery(
      data: MediaQueryData.fromView(t.view),
      child: Directionality(
        textDirection: TextDirection.ltr,
        child: AppTheme(
          colors: colors,
          formFactor: mobile ? FormFactor.mobile : FormFactor.desktop,
          child: DefaultTextStyle(
            style: ts(
              mobile ? TypeScale.mobile.body : TypeScale.desktop.body,
              colors.textPrimary,
            ),
            child: ColoredBox(
              color: colors.desk,
              child: Overlay(
                initialEntries: [
                  OverlayEntry(
                    builder: (_) => SizedBox.expand(child: presented),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    ),
  );
  await settle(t);
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

void main() {
  late _FakeShares hub;
  setUp(() {
    hub = _FakeShares();
    SharePopup.debugClientFor = (_) => hub;
  });
  tearDown(() => SharePopup.debugClientFor = null);

  testWidgets('the popup opens with Private and This version', (t) async {
    await pumpPopup(t);

    expect(key('share-popup'), findsOneWidget);
    expect(find.text('Share'), findsOneWidget);
    expect(find.text('Garden plan'), findsOneWidget);
    expect(key('share-visibility-public'), findsOneWidget);
    expect(key('share-visibility-private'), findsOneWidget);
    expect(key('share-kind-snapshot'), findsOneWidget);
    expect(key('share-kind-live'), findsOneWidget);
    expect(find.text('Who can view'), findsOneWidget);
    expect(find.text('What to share'), findsOneWidget);
    expect(find.text('Only people signed in to this hub can view.'), findsOneWidget);
    expect(find.text('A copy of the page as it is now.'), findsOneWidget);
    expect(key('share-create'), findsOneWidget);
    expect(find.text('Links to this page'), findsOneWidget);
    expect(find.text('No links yet.'), findsOneWidget);
  });

  testWidgets('without a hub sign-in the popup says so', (t) async {
    SharePopup.debugClientFor = (_) => null;
    await pumpPopup(t);

    expect(key('share-signed-out'), findsOneWidget);
    expect(key('share-create'), findsNothing);
  });

  testWidgets('creating a link sends the choices, copies and shows the URL', (t) async {
    final copied = recordClipboard(t);
    await pumpPopup(t);

    await tapAndSettle(t, key('share-visibility-public'));
    await tapAndSettle(t, key('share-kind-live'));
    expect(find.text('Anyone with the link can view.'), findsOneWidget);
    expect(find.text('Always shows the latest version.'), findsOneWidget);

    await tapAndSettle(t, key('share-create'));

    expect(hub.created, [(_page, 'public', 'live')]);
    expect(copied, ['https://hub.example.com/s/new1']);
    expect(t.widget<Text>(key('share-url')).data, 'https://hub.example.com/s/new1');
    expect(find.text('Copied'), findsOneWidget);
    await t.pump(const Duration(seconds: 3));
    expect(find.text('Copied'), findsNothing);
    expect(find.text('Copy'), findsWidgets);
    expect(key('share-row-new1'), findsOneWidget);
    expect(find.text('Public · Live link'), findsOneWidget);
  });

  testWidgets('the list shows each link with its kind and date', (t) async {
    final now = clockNow().millisecondsSinceEpoch;
    hub.shares.addAll([
      Share(
        id: 'a1',
        url: 'https://hub.example.com/s/a1',
        page: _page,
        visibility: 'public',
        kind: 'snapshot',
        created: now - 2 * _day,
      ),
      Share(
        id: 'a2',
        url: 'https://hub.example.com/s/a2',
        page: _page,
        visibility: 'private',
        kind: 'live',
        created: now - 20 * _day,
      ),
    ]);
    await pumpPopup(t);

    expect(
      find.descendant(
        of: key('share-row-a1'),
        matching: find.text('Public · This version'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: key('share-row-a1'),
        matching: find.text(relativeTime(now - 2 * _day)),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: key('share-row-a2'),
        matching: find.text('Private · Live link'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: key('share-row-a2'),
        matching: find.text(relativeTime(now - 20 * _day)),
      ),
      findsOneWidget,
    );
    expect(find.text('No links yet.'), findsNothing);
  });

  testWidgets('Copy copies a listed link', (t) async {
    final copied = recordClipboard(t);
    hub.shares.add(
      Share(
        id: 'a1',
        url: 'https://hub.example.com/s/a1',
        page: _page,
        visibility: 'public',
        kind: 'snapshot',
        created: clockNow().millisecondsSinceEpoch,
      ),
    );
    await pumpPopup(t);

    await tapAndSettle(t, key('share-copy-a1'));
    expect(copied, ['https://hub.example.com/s/a1']);
    expect(
      find.descendant(of: key('share-row-a1'), matching: find.text('Copied')),
      findsOneWidget,
    );
    await t.pump(const Duration(seconds: 3));
    expect(find.text('Copied'), findsNothing);
    expect(
      find.descendant(of: key('share-row-a1'), matching: find.text('Copy')),
      findsOneWidget,
    );
  });

  testWidgets('Revoke calls the hub and removes the row', (t) async {
    final now = clockNow().millisecondsSinceEpoch;
    hub.shares.addAll([
      Share(
        id: 'a1',
        url: 'https://hub.example.com/s/a1',
        page: _page,
        visibility: 'public',
        kind: 'snapshot',
        created: now,
      ),
      Share(
        id: 'a2',
        url: 'https://hub.example.com/s/a2',
        page: _page,
        visibility: 'private',
        kind: 'snapshot',
        created: now,
      ),
    ]);
    await pumpPopup(t);

    await tapAndSettle(t, key('share-revoke-a1'));
    expect(key('share-confirm-a1'), findsOneWidget);
    expect(
      find.text('Revoke this link? It stops working for everyone.'),
      findsOneWidget,
    );
    expect(hub.revoked, isEmpty);

    await tapAndSettle(t, key('share-revoke-confirm-a1'));

    expect(hub.revoked, ['a1']);
    expect(key('share-row-a1'), findsNothing);
    expect(key('share-row-a2'), findsOneWidget);
  });

  testWidgets('Cancel restores the row without calling the hub', (t) async {
    final now = clockNow().millisecondsSinceEpoch;
    hub.shares.add(
      Share(
        id: 'a1',
        url: 'https://hub.example.com/s/a1',
        page: _page,
        visibility: 'public',
        kind: 'snapshot',
        created: now,
      ),
    );
    await pumpPopup(t);

    await tapAndSettle(t, key('share-revoke-a1'));
    await tapAndSettle(t, key('share-cancel-a1'));

    expect(key('share-confirm-a1'), findsNothing);
    expect(key('share-row-a1'), findsOneWidget);
    expect(key('share-revoke-a1'), findsOneWidget);
    expect(hub.revoked, isEmpty);
  });

  testWidgets('an error from create shows an error line and keeps the popup open', (t) async {
    final copied = recordClipboard(t);
    hub.createError = "The hub didn't accept this device. Sign in to the hub again.";
    await pumpPopup(t);

    await tapAndSettle(t, key('share-create'));

    expect(key('share-error'), findsOneWidget);
    expect(
      t.widget<Text>(key('share-error')).data,
      "The hub didn't accept this device. Sign in to the hub again.",
    );
    expect(key('share-popup'), findsOneWidget);
    expect(key('share-url'), findsNothing);
    expect(copied, isEmpty);
    expect(hub.created, isEmpty);
  });
}
