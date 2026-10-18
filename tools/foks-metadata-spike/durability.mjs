// Private authority host boundary. An Ok kernel result is not an acknowledgment
// until both the encompassing SQL COMMIT and celld durable sync succeed.
// Pinned celld Date/performance are frozen during synchronous execution: these
// checks do NOT interrupt or bound a single SQL statement. After sync refreshes
// I/O time, an expired request must not publish its now-durable result.
// Awaited sync can still hang: this withholds publication, not a hard bound on
// request completion or proof that underlying storage work was cancelled.
export async function executeDurably(storage, execute, {deadline=Date.now()+1000,now=()=>Date.now()}={}) {
  const check=()=>{if(now()>=deadline)throw new Error('authority response deadline expired; outcome may be durable');};
  check();
  const result=storage.transactionSync(()=>{check();const value=execute();check();return value;});
  await storage.sync();
  check();
  return result;
}
