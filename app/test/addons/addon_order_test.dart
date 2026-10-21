import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/addon_registry.dart';

class TestAddon extends Addon {
  const TestAddon(this.id, [this.dependsOn = const []]);
  @override
  final String id;
  @override
  final List<String> dependsOn;
  @override
  void register(AddonContext ctx) {}
}

class RecordingAddon extends Addon {
  RecordingAddon(this.id, this.registered, [this.dependsOn = const []]);
  @override
  final String id;
  @override
  final List<String> dependsOn;
  final List<String> registered;
  @override
  void register(AddonContext ctx) => registered.add(id);
}

void main() {
  test(
    'stable dependency order, including transitive and shared dependencies',
    () {
      final ordered = orderAddons(const [
        TestAddon('board', ['database']),
        TestAddon('other'),
        TestAddon('database', ['document']),
        TestAddon('document'),
        TestAddon('extra', ['document']),
      ]);
      expect(ordered.map((a) => a.id), [
        'document',
        'database',
        'board',
        'other',
        'extra',
      ]);
    },
  );
  test('cycles include the dependency path', () {
    expect(
      () => orderAddons(const [
        TestAddon('a', ['b']),
        TestAddon('b', ['a']),
      ]),
      throwsA(
        isA<StateError>().having(
          (e) => e.message,
          'message',
          contains('a -> b -> a'),
        ),
      ),
    );
    expect(
      () => orderAddons(const [
        TestAddon('a', ['a']),
      ]),
      throwsStateError,
    );
  });
  test('missing dependencies fail before any add-on registers', () {
    final registered = <String>[];
    expect(
      () => bootAddons([
        RecordingAddon('first', registered),
        RecordingAddon('board', registered, ['database']),
      ], AddonContext()),
      throwsA(
        isA<StateError>().having(
          (e) => e.message,
          'message',
          contains('board -> database'),
        ),
      ),
    );
    expect(registered, isEmpty);
  });
  test('boot registers dependencies once before their dependents', () {
    final registered = <String>[];
    bootAddons([
      RecordingAddon('board', registered, ['database']),
      RecordingAddon('database', registered),
    ], AddonContext());
    expect(registered, ['database', 'board']);
  });
  test('duplicate ids fail', () {
    expect(
      () => orderAddons(const [TestAddon('a'), TestAddon('a')]),
      throwsStateError,
    );
  });
}
