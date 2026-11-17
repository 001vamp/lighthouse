"""Stage fake sessions in the live dock to see job/agent states, then clear them.
    dock-demo.py working|needs|failed|clear
Writes demo-* entries into $XDG_RUNTIME_DIR/toasty/sessions.json under its lock. Always run
`clear` after a screenshot. Set TOASTY_DEMO_JOB to a real T3 thread id to fold the running
orchestrator session into the demo job, or leave it for a standalone demo job."""
import fcntl, json, os, sys, time, uuid
P=os.path.join(os.environ['XDG_RUNTIME_DIR'], 'toasty', 'sessions.json')
JOB={'id':os.environ.get('TOASTY_DEMO_JOB','demo-job'),'title':'Install and Configure Git or GitHub CLI','open':''}
def s(agent, state, task, job=True):
    e={'agent':agent,'project':'demo','state':state,'revision':uuid.uuid4().hex,'time':int(time.time()),'app':'','task':task,'reason':''}
    if job: e['t3']={'thread':'demo','open':'','job':JOB}
    return e
with open(P+'.lock','w') as l:
    fcntl.flock(l, fcntl.LOCK_EX)
    d=json.load(open(P))
    for k in [k for k in d if k.startswith('demo-')]: d.pop(k)
    if sys.argv[1]=='working':
        d.update({'demo-c1':s('Claude Code','done','Write the README'),'demo-c2':s('Claude Code','working','Update the design doc'),
                  'demo-c3':s('Claude Code','working','Refresh the roadmap'),
                  'demo-a1':s('Codex','done','Fix the brightness slider',False),'demo-a2':s('Claude Code','working','Tidy up the installer',False)})
    elif sys.argv[1]=='needs':
        d.update({'demo-c1':s('Claude Code','done','Write the README'),'demo-c2':s('Claude Code','waiting','Gemini hooks'),
                  'demo-c3':s('Claude Code','working','Refresh the roadmap'),
                  'demo-a1':s('Codex','done','Fix the brightness slider',False),'demo-a2':s('Claude Code','working','Tidy up the installer',False)})
    elif sys.argv[1]=='failed':
        d.update({'demo-c1':s('Claude Code','done','Write the README'),'demo-c2':s('Claude Code','failed','Gemini hooks'),
                  'demo-c3':s('Claude Code','working','Refresh the roadmap'),
                  'demo-a1':s('Codex','paused','Fix the brightness slider',False),'demo-a2':s('Claude Code','waiting','Tidy up the installer',False)})
    t=P+'.tmp-demo'; json.dump(d,open(t,'w')); os.replace(t,P)
