use std::collections::HashMap;
#[derive(Debug, Clone)] pub struct Draft { pub id: String, pub revision: u64, pub content: String, pub expires_at: u64 }
#[derive(Debug, Default)] pub struct DraftStore { drafts: HashMap<String,Draft> }
impl DraftStore {
    pub fn create(&mut self, id: &str, content: String, now: u64) -> Result<(), String> {
        if self.drafts.contains_key(id) { return Err("DI_DRAFT_REVISION_CONFLICT".into()); }
        self.drafts.insert(id.into(), Draft{id:id.into(),revision:0,content,expires_at:now+3600});
        Ok(())
    }
    pub fn update(&mut self, id: &str, content: String, expected_rev: u64) -> Result<(), String> {
        let d=self.drafts.get_mut(id).ok_or("DI_DRAFT_NOT_FOUND".to_string())?;
        if d.revision != expected_rev { return Err("DI_DRAFT_REVISION_CONFLICT".into()); }
        d.revision+=1; d.content=content; Ok(())
    }
    pub fn get(&self,id:&str,now:u64)->Result<&Draft,String>{
        let d=self.drafts.get(id).ok_or("DI_DRAFT_NOT_FOUND".to_string())?;
        if now > d.expires_at { return Err("DI_DRAFT_EXPIRED".into()); }
        Ok(d)
    }
}
