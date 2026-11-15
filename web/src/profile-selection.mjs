export const gameSession=state=>state?.active_game_session??state?.active_game_profile_id??null;
/** Manual browsing is scoped to the current game session. A new login/account
 * automatically resumes following; ordinary live updates never undo a choice.
 */
export function selectedProfile(selection,state) {
  if(!state)return selection.profileId;
  const profiles=state?.profiles??[];
  const exists=id=>profiles.some(p=>p.profile_id===id);
  const active=state?.active_game_profile_id;
  const manual=selection.manualSession!==null&&selection.manualSession===gameSession(state);
  if(active&&exists(active)&&!manual)return active;
  if(exists(selection.profileId))return selection.profileId;
  return profiles[0]?.profile_id??'';
}
