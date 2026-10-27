"""Structural visual-basis validation; no style inference or quality scoring."""

VERSION = 'series-visual-baseline-v1'
COVERAGE = {'form', 'rendering', 'materials', 'colorLighting', 'aspectRatio', 'localExceptions'}


def validate_visual_baseline(value, adopted=True):
    """Return the exact supplied object, or raise ValueError on invalid evidence.

    Coverage values are excerpts of the actual style text, not proof that an
    artistic choice is sufficiently specific, consistent, or approved by a user.
    """
    required = {'version', 'id', 'revision', 'status', 'visualStyle', 'coverage'}
    optional = {'adoptionBasis'}
    if not isinstance(value, dict) or not required <= value.keys() or value.keys() - required - optional:
        raise ValueError('visual baseline: invalid fields')

    def text(item, label):
        if not isinstance(item, str) or not item.strip() or '\0' in item or '\r' in item:
            raise ValueError('visual baseline: nonempty text required for ' + label)

    if value['version'] != VERSION:
        raise ValueError('visual baseline: unsupported version')
    text(value['id'], 'id')
    if '\n' in value['id']:
        raise ValueError('visual baseline: id must be one line')
    if type(value['revision']) is not int or not 1 <= value['revision'] <= 9007199254740991:
        raise ValueError('visual baseline: revision must be a positive safe integer')
    if value['status'] not in ('candidate', 'adopted') or (adopted and value['status'] != 'adopted'):
        raise ValueError('visual baseline: an adopted baseline is required')
    text(value['visualStyle'], 'visualStyle')
    if not isinstance(value['coverage'], dict) or set(value['coverage']) != COVERAGE:
        raise ValueError('visual baseline: all six coverage excerpts are required')
    for name, excerpt in value['coverage'].items():
        text(excerpt, 'coverage.' + name)
        if excerpt not in value['visualStyle']:
            raise ValueError('visual baseline: coverage excerpt not in visualStyle: ' + name)
    if len(set(value['coverage'].values())) == 1:
        raise ValueError('visual baseline: one repeated label cannot supply six coverage dimensions')
    if value['status'] == 'adopted' or 'adoptionBasis' in value:
        text(value.get('adoptionBasis'), 'adoptionBasis')
    return value
