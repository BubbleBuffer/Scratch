// Legacy settings.json files written before add-on switches were retired. The
// `disabledAddons` list is ignored (every add-on is always on) and the next
// save drops it, while the other device settings load and save as before.
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/settings/settings_store.dart';

void main() {
  late Directory dir;
  late File file;

  setUp(() {
    dir = Directory.systemTemp.createTempSync('notes-legacy-settings-');
    file = File('${dir.path}/settings.json');
  });

  tearDown(() => dir.deleteSync(recursive: true));

  Map<String, dynamic> saved() =>
      jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;

  test('a legacy disabledAddons list does not change the other settings', () {
    file.writeAsStringSync(
      jsonEncode({
        'disabledAddons': ['board', 'agent', 'tasks'],
        'theme': 'dark',
        'scheduleRunsHere': true,
      }),
    );
    final settings = AppSettings.load(file);
    expect(settings.theme, ThemePref.dark);
    expect(settings.scheduleRunsHere, isTrue);
    settings.dispose();
  });

  test('the next save drops the legacy disabledAddons key', () {
    file.writeAsStringSync(
      jsonEncode({
        'disabledAddons': ['board'],
        'theme': 'light',
        'hubAddress': '127.0.0.1:47100',
      }),
    );
    final settings = AppSettings.load(file);
    settings.update((s) => s.prefetchAttachments = true);
    final json = saved();
    expect(json.containsKey('disabledAddons'), isFalse);
    expect(json['theme'], 'light');
    expect(json['hubAddress'], '127.0.0.1:47100');
    expect(json['prefetchAttachments'], isTrue);
    settings.dispose();
  });

  test(
    'a legacy file with an empty disabledAddons list also drops the key',
    () {
      file.writeAsStringSync(jsonEncode({'disabledAddons': <String>[]}));
      final settings = AppSettings.load(file);
      settings.update((s) => s.shakeToSend = false);
      expect(saved().containsKey('disabledAddons'), isFalse);
      expect(saved()['shakeToSend'], isFalse);
      settings.dispose();
    },
  );

  test('new settings files never gain the key', () {
    final settings = AppSettings(file: file);
    settings.update((s) => s.theme = ThemePref.dark);
    expect(saved().containsKey('disabledAddons'), isFalse);
    expect(AppSettings.load(file).theme, ThemePref.dark);
    settings.dispose();
  });
}
