"""Independent replica of the selected renderer geometry, normalized to 800x450.
No rendering or artifact modification occurs here.
"""
import numpy as np, math, json, argparse
W,H=800,450;sx,sy=W/1672,H/941
cx,cy=834.5*sx,435.5*sy;rx=208.5*sx
points=[[[838,58],[895,179],[781,179]],[[626,126],[738,190],[649,252]],[[1051,127],[1026,252],[937,190]],[[482,294],[621,287],[576,391]],[[1051,287],[1190,294],[1095,392]]]
P=np.array(points,dtype=float)*[sx,sy];C=P.mean(axis=1);LOCAL=P-C[:,None,:]
TIP=np.array([math.atan2(*(P[i,0 if i<4 else 1]-C[i])[::-1]) for i in range(5)])
curves=np.array([[[534,433],[458,619],[578,847],[833,849]],[[833,849],[1063,861],[1200,641],[1136,433]],[[1136,433],[1114,611],[1011,698],[835,697]],[[835,697],[656,698],[553,602],[534,433]]],dtype=float)*[sx,sy]
def crescent(n=96):
 t=np.arange(n)/n;s=1-t
 return np.concatenate([s[:,None]**3*c[0]+3*(s*s*t)[:,None]*c[1]+3*(s*t*t)[:,None]*c[2]+t[:,None]**3*c[3] for c in curves])
def triangles(R,tilt,up,n=96,nominal_scale=.94):
 phase=np.arange(n)*2*np.pi/n;ids=np.arange(5)
 theta=np.pi/2+phase[:,None]+ids[None,:]*2*np.pi/5
 ti=tilt*np.pi/180
 x3=R*np.cos(theta);y3=R*np.sin(theta)*np.cos(ti);z=R*np.sin(theta)*np.sin(ti)
 perspective=1/(1-z/(rx*7.5));px=x3*perspective;py=-y3*perspective-up
 scale=perspective*nominal_scale;yaw=.82+.18*np.abs(np.sin(theta))
 rot=np.arctan2(-y3*perspective,px)-TIP[None,:]+.055*np.sin(2*phase[:,None]+ids[None,:])
 ux=LOCAL[None,:,:,0]*yaw[:,:,None];uy=LOCAL[None,:,:,1]
 xx=(ux*np.cos(rot)[:,:,None]-uy*np.sin(rot)[:,:,None])*scale[:,:,None]+px[:,:,None]+cx
 yy=(ux*np.sin(rot)[:,:,None]+uy*np.cos(rot)[:,:,None])*scale[:,:,None]+py[:,:,None]+cy
 return np.stack([xx,yy],axis=-1).reshape((-1,3,2)),np.stack([px+cx,py+cy,z],axis=-1).reshape((-1,3))
def cross(a,b):return a[...,0]*b[...,1]-a[...,1]*b[...,0]
def point_seg(p,a,b):
 v=b-a;w=p-a;t=np.clip(np.sum(w*v,axis=-1)/np.sum(v*v,axis=-1),0,1)
 return np.sum((w-t[...,None]*v)**2,axis=-1)
def sep(tris,moon):
 A=tris[:,:,None,:];B=np.roll(tris,-1,axis=1)[:,:,None,:]
 C=moon[None,None,:,:];D=np.roll(moon,-1,axis=0)[None,None,:,:]
 d=np.minimum(np.minimum(point_seg(A,C,D),point_seg(B,C,D)),np.minimum(point_seg(C,A,B),point_seg(D,A,B)))
 a=cross(B-A,C-A);b=cross(B-A,D-A);c=cross(D-C,A-C);e=cross(D-C,B-C)
 intersects=((a*b<0)&(c*e<0)).any(axis=(1,2))
 mins=np.sqrt(d.min(axis=(1,2)));mins[intersects]=0
 # Containment matters even when boundaries have no intersection.
 q=tris[:,0,None,:];p=moon[None,:,:];r=np.roll(moon,-1,axis=0)[None,:,:]
 dy=r[...,1]-p[...,1];xd=(r[...,0]-p[...,0])*(q[...,1]-p[...,1])/np.where(dy==0,1e-20,dy)+p[...,0]
 inside=np.logical_xor.reduce(((p[...,1]>q[...,1])!=(r[...,1]>q[...,1]))&(q[...,0]<xd),axis=1)
 mins[inside]=0
 return mins
