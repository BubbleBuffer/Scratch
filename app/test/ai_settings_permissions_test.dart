import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/ai/ai_settings.dart';

void main() {
  late Directory dir;
  late File file;
  setUp(() {
    dir = Directory.systemTemp.createTempSync('notes-permissions-');
    file = File('${dir.path}/ai_settings.json');
  });
  tearDown(() => dir.deleteSync(recursive: true));

  test('new installs keep Auto as a dormant preference', () {
    expect(AiSettings().agentPermissionMode, AgentPermissionMode.auto);
    expect(AiSettings.load(file).agentPermissionMode, AgentPermissionMode.auto);
  });

  for (final legacy in {'ask': AgentPermissionMode.manual, 'allow': AgentPermissionMode.bypass}.entries) {
    test('preserves legacy ${legacy.key} as a dormant preference and keeps the old root', () {
      file.writeAsStringSync(jsonEncode({'agentApprovals': legacy.key, 'desktopRoot': '/old/root'}));
      final settings = AiSettings.load(file);
      expect(settings.agentPermissionMode, legacy.value);
      expect(settings.desktopRoot, '/old/root');
      settings.update((_) {});
      expect(AiSettings.load(file).agentPermissionMode, legacy.value);
      expect(AiSettings.load(file).desktopRoot, '/old/root');
    });
  }

  test('the saved mode takes precedence over the legacy key', () {
    file.writeAsStringSync(jsonEncode({'agentPermissionMode': 'auto', 'agentApprovals': 'allow'}));
    expect(AiSettings.load(file).agentPermissionMode, AgentPermissionMode.auto);
  });
}
