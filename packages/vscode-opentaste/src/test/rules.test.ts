import { strict as assert } from 'node:assert';
import { test } from 'node:test';

import { countEnabledRules, learnedMessage } from '../rules';
import { isGeneratedTarget, parseTargets } from '../targets';

test('the status count is the enabled confirmed rules', () => {
  const file = JSON.stringify({ rules: [{ enabled: true }, { enabled: false }, { enabled: true }] });
  assert.equal(countEnabledRules(file), 2);
  assert.equal(countEnabledRules('{"rules":[]}'), 0);
  assert.equal(countEnabledRules('not json'), undefined);
  assert.equal(countEnabledRules('{}'), undefined);
});

test('the toast reads as SPEC.md section 9 writes it', () => {
  assert.equal(
    learnedMessage('Use @AutoEntity() instead of @Entity()'),
    '✨ OpenTaste learned: "Use @AutoEntity() instead of @Entity()"',
  );
});

test('taste targets --json parses, and anything else means do not learn', () => {
  const targets = parseTargets(
    '{"marker_targets":["CLAUDE.md",".windsurfrules","AGENTS.md",".github/copilot-instructions.md"],' +
      '"cursor_rules_dir":".cursor/rules","cursor_rule_prefix":"opentaste-","cursor_rule_extension":"mdc"}\n',
  );
  assert.ok(targets);
  assert.ok(isGeneratedTarget('.cursor/rules/opentaste-types.mdc', targets));
  assert.ok(isGeneratedTarget('.\\CLAUDE.md'.replace('.\\', ''), targets));
  assert.ok(!isGeneratedTarget('.cursor/rules/team.mdc', targets));
  assert.ok(!isGeneratedTarget('docs/CLAUDE.md', targets));
  assert.equal(parseTargets('{"marker_targets":"CLAUDE.md"}'), undefined);
  assert.equal(parseTargets(''), undefined);
});
