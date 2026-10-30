// Connects the remote-control endpoint to the running app: pages and
// navigation through the app's own navigator and stores, the screenshot from
// the feedback screenshot boundary, the UI dump and actions from the
// semantics tree.

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../core/paths.dart';
import '../feedback/feedback_host.dart';
import '../features/settings/settings_screen.dart';
import '../shell/navigation.dart';
import '../shell/search.dart';
import '../state/app_state.dart';
import '../ui/ui.dart';
import 'key_press.dart';
import 'remote_handler.dart';
import 'remote_server.dart';
import 'semantics_driver.dart';

/// The server and the semantics hold of a running remote endpoint.
class AppRemote {
  AppRemote._(this.server, this._driver);

  final RemoteControlServer server;
  final SemanticsDriver _driver;

  /// Starts the endpoint for the app whose state [app] returns (null until
  /// the vault is open).
  static Future<AppRemote> start({
    required String dataDir,
    required AppState? Function() app,
    required String version,
  }) async {
    final driver = SemanticsDriver()..enable();
    try {
      final server = await RemoteControlServer.start(
        backend: AppRemoteBackend(app: app, driver: driver),
        dataDir: dataDir,
        version: version,
      );
      AppLog.info('remote control on 127.0.0.1:${server.port}');
      return AppRemote._(server, driver);
    } catch (_) {
      driver.disable();
      rethrow;
    }
  }

  Future<void> stop() async {
    await server.stop();
    _driver.disable();
  }
}

class AppRemoteBackend implements RemoteBackend {
  AppRemoteBackend({required this.app, required this.driver});

  final AppState? Function() app;
  final SemanticsDriver driver;

  /// A context below the app navigator and above nothing the app hides:
  /// stores, the page stack and the feedback host are all above it.
  BuildContext _context() {
    final ctx = appNavigatorKey.currentState?.overlay?.context;
    if (ctx == null) throw const RemoteError(500, 'the app has no window yet');
    return ctx;
  }

  static String _folderName(String root) {
    final parts = root.split(RegExp(r'[\\/]')).where((s) => s.isNotEmpty);
    return parts.isEmpty ? root : parts.last;
  }

  AppState _app() {
    final a = app();
    if (a == null) throw const RemoteError(404, 'no vault is open');
    return a;
  }

  Future<({String id, String title})?> _titleOf(String id) async {
    try {
      final page = await _app().vault.loadPage(id);
      if (page.inTrash) return null;
      return (id: page.id, title: page.title);
    } catch (_) {
      return null;
    }
  }

  @override
  Future<Map<String, Object?>> status() async {
    final a = app();
    Map<String, Object?>? page;
    final last = a?.ui.lastPage;
    if (a != null && last != null) {
      final found = await _titleOf(last);
      if (found != null) page = {'id': found.id, 'title': found.title};
    }
    return {
      'vault': {
        'open': a != null,
        'name': a == null ? null : _folderName(a.vault.root),
      },
      'page': page,
    };
  }

  @override
  Future<Map<String, Object?>> open(String page) async {
    var found = await _titleOf(page);
    if (found == null) {
      final hits = await _app().vault.search(page);
      for (final h in hits) {
        if (h.source == 'vault' && h.title == page) {
          found = await _titleOf(h.id);
          if (found != null) break;
        }
      }
    }
    if (found == null) {
      throw RemoteError(404, 'no page with id or exact title "$page"');
    }
    final ctx = _context();
    unawaited(
      // ignore: use_build_context_synchronously
      openPage(ctx, found.id, found.title).catchError((Object e) {
        AppLog.info('remote open failed: $e');
      }),
    );
    return {'id': found.id, 'title': found.title};
  }

  @override
  Future<void> navigate(String route) async {
    // ignore: use_build_context_synchronously
    final ctx = _context();
    if (route.startsWith('page/')) {
      await open(route.substring(5));
      return;
    }
    switch (route) {
      case 'home':
        goHome(ctx);
      case 'settings':
        unawaited(showSettings(ctx));
      case 'search':
        unawaited(
          pushScreen<void>(ctx, (_) => const SearchScreen(), name: '/search'),
        );
      case 'back':
        unawaited(appNavigatorKey.currentState!.maybePop());
      default:
        throw RemoteError(400, 'unknown route "$route"');
    }
  }

  @override
  Future<RemoteImage> screenshot(double pixelRatio) async {
    final key = FeedbackHost.of(_context()).captureKey;
    final box = key.currentContext?.findRenderObject();
    if (box is! RenderRepaintBoundary) {
      throw const RemoteError(500, 'no frame to capture');
    }
    final image = await box.toImage(pixelRatio: pixelRatio);
    try {
      final data = await image.toByteData(format: ui.ImageByteFormat.png);
      if (data == null) throw const RemoteError(500, 'could not encode the frame');
      return RemoteImage(
        width: image.width,
        height: image.height,
        png: data.buffer.asUint8List(),
      );
    } finally {
      image.dispose();
    }
  }

  @override
  Future<List<Map<String, Object?>>> tree() async => [
    for (final n in driver.snapshot()) n.toJson(),
  ];

  @override
  Future<void> tap(String id) async => driver.tap(id);

  @override
  Future<void> type(String id, String text, {required bool replace}) =>
      driver.type(id, text, replace: replace);

  @override
  Future<void> key(KeyCombo combo) async => pressKeyCombo(combo);
}
