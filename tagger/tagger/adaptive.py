"""Evidence-controlled reach for every mood, with nested song exclusion.

Local corrections are the conservative default. Broader local/global fits must
improve predictions for excluded songs. Imports and inference never refit.
"""
import base64
import hashlib

import numpy as np
from scipy.optimize import minimize
from scipy.special import expit

from . import personal


def refine(prior, geometry, flags, reference_geometry, comparisons=()):
    """Replace a local residual over an immutable accepted foundation.

    No model selection or global refit. Repeating the same feedback is
    idempotent; removing a flag removes its residual, rather than stacking
    another correction on the last preview. Unjudged imports are not anchors.
    """
    prior = np.asarray(prior, dtype=float)
    reference_geometry = np.asarray(reference_geometry,dtype=float)
    if (reference_geometry.ndim != 2 or reference_geometry.shape[1] != geometry.shape[1]
            or len(reference_geometry) < 2 or not np.isfinite(reference_geometry).all()):
        raise ValueError('Feedback correction needs matching fixed reference audio geometry')
    ids = sorted({f.index for f in flags} | {i for pair in comparisons for i in pair})
    if not ids:
        return None, personal.smooth_score(prior)
    width = .05
    K = np.exp(-np.maximum(0, 1-geometry@geometry[ids].T)/width)
    gram = K[ids]+np.eye(len(ids))*1e-6
    eigenvalues, eigenvectors = np.linalg.eigh(gram)
    transform = eigenvectors/np.sqrt(np.maximum(eigenvalues, 1e-6))
    rows = [f.index for f in flags]
    A = K[rows]@transform
    anchor = np.exp(-np.maximum(0, 1-reference_geometry@geometry[ids].T)/width)@transform
    weights = np.array([f.weight*f.confidence/4 for f in flags])
    D = np.array([K[a]-K[b] for a,b in comparisons]).reshape((-1,len(ids)))@transform
    pair_prior = np.array([prior[a]-prior[b] for a,b in comparisons])

    def objective(coef):
        losses, gradient = personal.directional_losses(prior[rows]+A@coef, flags)
        pair = pair_prior+D@coef
        cost = .02*np.mean(np.logaddexp(0,-pair)) if len(D) else 0.
        grad = -.02*D.T@expit(-pair)/len(D) if len(D) else 0.
        drift = anchor@coef
        return (float(weights@losses+.001*(coef@coef)+cost+.03*np.mean(drift**2)),
                A.T@(weights*gradient)+.002*coef+grad+.06*anchor.T@drift/len(anchor))

    result = minimize(objective, np.zeros(len(ids)), jac=True, method='L-BFGS-B',
                      options={'maxiter':1500,'ftol':1e-12,'gtol':1e-8})
    if not result.success:
        result = minimize(objective,result.x,jac=True,method='SLSQP',options={'maxiter':1500,'ftol':1e-11})
    if not result.success:
        raise ValueError('Feedback correction did not converge; saved scoring is unchanged')
    correction = personal.LocalCorrection(width=width,dimensions=geometry.shape[1],
        coefficients=(transform@result.x).tolist(),
        vectors=base64.b64encode(geometry[ids].astype('<f4').tobytes()).decode('ascii'))
    return correction, personal.smooth_score(prior+personal.local_latent(correction,geometry))


def validation_folds(flags, rows, geometry):
    """Five deterministic audio groups, independent of labels and library order.

    Identical audio vectors are always excluded together. Nested folds bound
    fitting cost while every judgment still receives an excluded prediction.
    """
    groups = {}
    for i in rows:
        key = hashlib.sha256(geometry[flags[i].index].astype('<f4').tobytes()).digest()
        groups.setdefault(key,[]).append(i)
    folds = [[] for _ in range(min(5,len(groups)))]
    for i,key in enumerate(sorted(groups)):
        folds[i%len(folds)].extend(groups[key])
    return folds


def fit(features, hidden, flags, signs, audio_count, *, reference_features,
        reference_geometry, comparisons=(), validate=True, diagnostics=None, select=True):
    geometry = personal.audio_geometry(hidden)
    scale = np.pi*1.8/4
    base = np.r_[-.674*scale,np.zeros(audio_count),[scale*s/max(1,signs.count(s)) for s in signs]]
    X = np.column_stack([np.ones(len(features)),features])
    prior = X@base
    support_ids = sorted({f.index for f in flags}|{i for pair in comparisons for i in pair})
    support_columns = {row:i for i,row in enumerate(support_ids)}
    distance = np.maximum(0,1-geometry@geometry[support_ids].T)
    reference_geometry = np.asarray(reference_geometry,dtype=float)
    if reference_geometry.ndim!=2 or reference_geometry.shape[1]!=geometry.shape[1] or len(reference_geometry)<2 or not np.isfinite(reference_geometry).all():
        raise ValueError('This fit needs matching fixed reference audio geometry')
    reference_distance = np.maximum(0,1-reference_geometry@geometry[support_ids].T)
    memo = {}
    kernels = {width:np.exp(-distance/width) for width in (.05,.1,.3)}
    reference_kernels = {width:np.exp(-reference_distance/width) for width in kernels}

    def train(rows, candidate):
        key = (tuple(rows),candidate)
        if key in memo: return memo[key]
        method,parameter = candidate
        omitted = {f.index for i,f in enumerate(flags) if i not in rows}
        pairs = [(a,b) for a,b in comparisons if a not in omitted and b not in omitted]
        fs = [flags[i] for i in rows]
        if method == 'global':
            weights,score,_ = personal.fit_directional(features,fs,signs,audio_count,
                reference_features=reference_features,comparisons=pairs,validate=False,
                fixed_regularization=parameter)
            result = (weights,None,personal.smooth_score(X@weights))
        else:
            ids = sorted({f.index for f in fs}|{i for pair in pairs for i in pair})
            if not ids:
                result = (base,None,personal.smooth_score(prior))
            else:
                K = kernels[parameter][:,[support_columns[i] for i in ids]]
                gram = K[ids]+np.eye(len(ids))*1e-6
                A = K[[f.index for f in fs]]
                eigenvalues,eigenvectors = np.linalg.eigh(gram)
                transform = eigenvectors/np.sqrt(np.maximum(eigenvalues,1e-6))
                A = A@transform
                anchor = reference_kernels[parameter][:,[support_columns[i] for i in ids]]@transform
                directions = {f.polarity for f in fs if f.pivot==.5 and f.polarity is not None}
                anchor_strength = .03 if directions=={-1,1} else .3
                weights = np.array([f.weight*f.confidence/4 for f in fs])
                D = np.array([K[a]-K[b] for a,b in pairs]).reshape((-1,len(ids)))
                D = D@transform
                pair_prior = np.array([prior[a]-prior[b] for a,b in pairs])

                def objective(coef):
                    losses,gradient = personal.directional_losses(prior[[f.index for f in fs]]+A@coef,fs)
                    pair = pair_prior+D@coef
                    cost = .02*np.mean(np.logaddexp(0,-pair)) if len(D) else 0.
                    grad = -.02*D.T@expit(-pair)/len(D) if len(D) else 0.
                    drift = anchor@coef
                    return float(weights@losses+.001*(coef@coef)+cost+anchor_strength*np.mean(drift**2)), A.T@(weights*gradient)+.002*coef+grad+2*anchor_strength*anchor.T@drift/len(anchor)

                result_ = minimize(objective,np.zeros(len(ids)),jac=True,method='L-BFGS-B',
                    options={'maxiter':1500,'ftol':1e-12,'gtol':1e-8})
                if not result_.success:
                    result_ = minimize(objective,result_.x,jac=True,method='SLSQP',options={'maxiter':1500,'ftol':1e-11})
                if not result_.success: raise ValueError('Local feedback fit did not converge; saved scoring is unchanged')
                coef = transform@result_.x
                local = (parameter,ids,coef)
                result = (base,local,personal.smooth_score(prior+K@coef))
        memo[key] = result
        return result

    def losses(rows,candidate):
        output = {}
        for fold in validation_folds(flags,rows,geometry):
            values = train([j for j in rows if j not in fold],candidate)[2]
            for i in fold:
                latent = np.tan(np.pi*(values[flags[i].index]-.5))
                output[i] = personal.directional_losses([latent],[flags[i]])[0][0]
        return np.array([output[i] for i in rows])

    def choose(rows):
        # Positive examples alone establish similarity, not a global boundary.
        directions = {flags[i].polarity for i in rows if flags[i].pivot==.5 and flags[i].polarity is not None}
        if len(rows)<5 or directions!={-1,1}: return ('local',.05)
        candidates = [('local',v) for v in (.05,.1,.3)]+[('global',v) for v in (.1,.01,.0001)]
        errors = {candidate:losses(rows,candidate) for candidate in candidates}
        weight = np.array([flags[i].weight*flags[i].confidence for i in rows],float)
        weight /= weight.sum()
        best = min(candidates,key=lambda c:float(weight@errors[c]))
        prior_loss = personal.directional_losses([prior[flags[i].index] for i in rows],[flags[i] for i in rows])[0]
        if float(weight@errors[best])>=float(weight@prior_loss): return ('local',.05)
        # One paired standard error: prefer narrower reach when evidence is tied.
        for candidate in candidates:
            diff = errors[candidate]-errors[best]
            mean = float(weight@diff)
            se = np.sqrt(float(weight@((diff-mean)**2))*float(weight@weight))
            if mean<=se+1e-12: return candidate
        return best

    rows = list(range(len(flags)))
    selected = choose(rows) if select else ('local',.05)
    held = {}
    if validate and len(rows)>=5:
        for fold in validation_folds(flags,rows,geometry):
            remaining = [j for j in rows if j not in fold]
            values = train(remaining,choose(remaining))[2]
            for i in fold:
                latent = np.tan(np.pi*(values[flags[i].index]-.5))
                held[i] = personal.directional_losses([latent],[flags[i]])[0][0]
    weights,local,values = train(rows,selected)
    correction = None
    if local is not None:
        width,ids,coef = local
        correction = personal.LocalCorrection(width=width,dimensions=geometry.shape[1],
            coefficients=coef.tolist(),vectors=base64.b64encode(geometry[ids].astype('<f4').tobytes()).decode('ascii'))
        # Saved binary vectors are normalized again by inference. Verify that
        # encoding cannot silently introduce a different preview.
        values = personal.smooth_score(X@weights+personal.local_latent(correction,geometry))
    if diagnostics is not None:
        diagnostics.update(method=selected[0] if rows or comparisons else 'words',local=correction,
            validation_loss=float(np.average([held[i] for i in rows],weights=[f.weight*f.confidence for f in flags])) if held else None)
    return weights,values,(0,0)
