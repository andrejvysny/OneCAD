# Independent design arithmetic checks

**Executed during document preparation:** 13 September 2026, Python/NumPy in the document-authoring environment.  
**Not executed:** OneCAD, its native worker, its browser renderer, or its application tests.

These checks validate selected algebraic identities and numerical counterexamples in NUM. They do not prove the implementation correct, certify arbitrary OCCT curves, or establish GPU performance. Required native/GPU gates remain not-run.

## Observed output

```text
rational_derivative_numerator_max_abs_error 1.14e-13
projected_segment_reconstruction_max_abs_error 1.78e-15
screen_linear_dash_pair_max_abs_error 1.14e-13
s_curve_t_0_25_y -1.125
jacobian_largest_singular_value 1.618033988749895
legacy_float32_spacing_mm 0.0625 1.0 64.0
clamped_zoom_effective_factor 1.0
circle_1000_px_64_segments_sagitta 1.204543794827595
legacy_6px_depth_bias_mm 2.437611154700958
```

The derivative check uses degrees 2–6, 20 deterministic positive-weight random spans per degree, and 31 parameter samples per span. The perspective and dash identities use 1,000 deterministic positive-depth examples. Errors shown are floating-point identity discrepancies for those samples, not geometric approximation guarantees.

## Reproduce

Run the following code in an environment with Python 3 and NumPy. It is an isolated design check; port the relevant cases into the application's normal test lanes rather than treating this script as their replacement.

```python
import numpy as np
from math import comb,pi,cos,tan

def bern_eval(c,t):
    n=len(c)-1
    return sum(comb(n,i)*(1-t)**(n-i)*t**i*c[i] for i in range(n+1))

def bern_product(a,b):
    # Scalars b, vectors a; enough for both derivative-numerator products.
    m,n=len(a)-1,len(b)-1
    out=[]
    for k in range(m+n+1):
        out.append(sum(comb(m,i)*comb(n,k-i)/comb(m+n,k)*a[i]*b[k-i]
                       for i in range(max(0,k-n),min(m,k)+1)))
    return np.array(out)

rng=np.random.default_rng(90213)
max_q=0.0
for n in range(2,7):
    for _ in range(20):
        p=rng.uniform(-4,4,(n+1,3));w=rng.uniform(.3,3,n+1);x=p*w[:,None]
        dx=n*np.diff(x,axis=0);dw=n*np.diff(w)
        q=bern_product(dx,w)-bern_product(x,dw)
        for t in np.linspace(0,1,31):
            reference=bern_eval(dx,t)*bern_eval(w,t)-bern_eval(x,t)*bern_eval(dw,t)
            max_q=max(max_q,float(np.max(np.abs(reference-bern_eval(q,t)))))
assert max_q<1e-10

max_proj=0.0;max_dash=0.0
for _ in range(1000):
    a,b=sorted(rng.uniform(.2,20,2));x0,x1=rng.uniform(-8,8,2)
    z0,z1=a,b;l=rng.uniform(0,1)
    t=(l/z1)/((1-l)/z0+l/z1)
    x=(1-t)*x0+t*x1;z=(1-t)*z0+t*z1
    screen_ref=(1-l)*x0/z0+l*x1/z1
    max_proj=max(max_proj,abs(x/z-screen_ref))
    d0,d1=rng.uniform(0,300,2)
    aa,bb=(1-l)/z0,l/z1
    # Interpolation of the pair (distance*clipW, clipW), then ratio.
    first=(aa*d0*z0+bb*d1*z1)/(aa+bb)
    second=(aa*z0+bb*z1)/(aa+bb)
    max_dash=max(max_dash,abs(first/second-((1-l)*d0+l*d1)))
assert max_proj<1e-12 and max_dash<1e-10
poles=np.array([[0,0,0],[.2,0,0],[.4,-6.4,0],[.6,6.4,0],[.8,0,0],[1,0,0]])
s=bern_eval(poles,.25)
assert abs(s[1]+1.125)<1e-12
j=np.array([[1.,1.],[0.,1.]])
a,b=np.dot(j[:,0],j[:,0]),np.dot(j[:,0],j[:,1]);d=np.dot(j[:,1],j[:,1])
sig_formula=np.sqrt(.5*(a+d+np.sqrt((a-d)**2+4*b*b)))
assert abs(sig_formula-np.linalg.svd(j,compute_uv=False)[0])<1e-12
old_h=.5;requested=.5;new_h=max(.5,min(50000,old_h*requested));effective=new_h/old_h
assert effective==1
spacing1=float(np.spacing(np.float32(1e6)));spacing2=float(np.spacing(np.float32(1e7)));spacing3=float(np.spacing(np.float32(1e9)))
assert spacing1==.0625 and spacing2==1.0 and spacing3==64.0
print('rational_derivative_numerator_max_abs_error',format(max_q,'.3g'))
print('projected_segment_reconstruction_max_abs_error',format(max_proj,'.3g'))
print('screen_linear_dash_pair_max_abs_error',format(max_dash,'.3g'))
print('s_curve_t_0_25_y',s[1])
print('jacobian_largest_singular_value',sig_formula)
print('legacy_float32_spacing_mm',spacing1,spacing2,spacing3)
print('clamped_zoom_effective_factor',effective)
print('circle_1000_px_64_segments_sagitta',1000*(1-cos(pi/64)))
print('legacy_6px_depth_bias_mm',6*2*260*tan(76*pi/360)/1000)
```
