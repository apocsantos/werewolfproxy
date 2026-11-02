"""Layer B findings (Stage38 final D1 rules), from one experiment's bounds.

Two independent findings, both always reported:
  non-inferiority: LIVENESS_NONINFERIOR (U < delta), LIVENESS_INFERIOR
                   (L > delta), otherwise LIVENESS_INCONCLUSIVE;
  WORSENING_DETECTED whenever L > 0.
With Layer A HELD, VIOLATED or INCOMPLETE the comparison is blocked: the
estimates may be kept as diagnostics, never as an approved result.
"""

BLOCKING = ('HELD', 'VIOLATED', 'INCOMPLETE')


def findings(u_below_delta, l_above_delta, l_above_zero, layer_a='SATISFIED', status_ok=True):
    if layer_a in BLOCKING:
        return {'layer_b': 'BLOCKED', 'worsening': None, 'diagnostic_only': True}
    if u_below_delta and l_above_delta:
        raise ValueError('U < delta and L > delta cannot both hold')
    if not status_ok:
        ni = 'LIVENESS_INCONCLUSIVE'
    elif u_below_delta:
        ni = 'LIVENESS_NONINFERIOR'
    elif l_above_delta:
        ni = 'LIVENESS_INFERIOR'
    else:
        ni = 'LIVENESS_INCONCLUSIVE'
    return {'layer_b': ni, 'worsening': bool(status_ok and l_above_zero) and 'WORSENING_DETECTED' or None,
            'diagnostic_only': False}
