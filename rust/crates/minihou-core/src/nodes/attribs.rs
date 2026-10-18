//! Attribute Delete and Attribute Rename (nodes/attribs.py).

/// Python fnmatchcase's *, ?, [abc], [!abc], and ranges, with literal unmatched [.
pub(crate) fn matches(pattern:&str,name:&str)->bool{
    fn go(p:&[char],s:&[char])->bool{
        if p.is_empty(){return s.is_empty();}
        match p[0]{
            '*'=> (0..=s.len()).any(|i|go(&p[1..],&s[i..])),
            '?'=> !s.is_empty() && go(&p[1..],&s[1..]),
            '['=>{
                let start=1+usize::from(p.get(1)==Some(&'!'));
                let search=start+usize::from(p.get(start)==Some(&']'));
                if let Some(end)=(search..p.len()).find(|&i|p[i]==']'){
                    if s.is_empty(){return false;}let mut hit=false;let mut i=start;
                    while i<end{if i+2<end && p[i+1]=='-'{hit|=p[i]<=s[0] && s[0]<=p[i+2];i+=3;}else{hit|=p[i]==s[0];i+=1;}}
                    if start==2{hit=!hit;}hit && go(&p[end+1..],&s[1..])
                }else{!s.is_empty() && s[0]=='[' && go(&p[1..],&s[1..])}
            }
            c=> !s.is_empty() && c==s[0] && go(&p[1..],&s[1..]),
        }
    }go(&pattern.chars().collect::<Vec<_>>(),&name.chars().collect::<Vec<_>>())
}
