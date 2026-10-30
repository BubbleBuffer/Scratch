// A [RemoteBackend] that records calls and returns canned answers.

import 'package:notes_app/remote/remote_handler.dart';

class FakeBackend implements RemoteBackend {
  final calls = <String>[];

  /// Set to make the next call throw.
  Object? failWith;

  void _call(String what) {
    calls.add(what);
    final f = failWith;
    if (f != null) throw f;
  }

  @override
  Future<Map<String, Object?>> status() async {
    _call('status');
    return {
      'vault': {'open': true, 'name': 'vault'},
      'page': {'id': 'p1', 'title': 'Plans'},
    };
  }

  @override
  Future<Map<String, Object?>> open(String page) async {
    _call('open $page');
    if (page == 'missing') throw const RemoteError(404, 'no page');
    return {'id': 'p1', 'title': 'Plans'};
  }

  @override
  Future<void> navigate(String route) async => _call('navigate $route');

  @override
  Future<RemoteImage> screenshot(double pixelRatio) async {
    _call('screenshot $pixelRatio');
    return const RemoteImage(width: 2, height: 3, png: [1, 2, 3]);
  }

  @override
  Future<List<Map<String, Object?>>> tree() async {
    _call('tree');
    return [
      {
        'id': 'save',
        'role': 'button',
        'label': 'Save',
        'value': '',
        'hint': '',
        'actions': ['tap'],
        'enabled': true,
        'focused': false,
        'rect': {'x': 1.0, 'y': 2.0, 'w': 3.0, 'h': 4.0},
      },
    ];
  }

  @override
  Future<void> tap(String id) async {
    _call('tap $id');
    if (id == 'gone') throw const RemoteError(404, 'no element');
  }

  @override
  Future<void> type(String id, String text, {required bool replace}) async =>
      _call('type $id "$text" replace=$replace');

  @override
  Future<void> key(KeyCombo combo) async => _call('key $combo');
}
