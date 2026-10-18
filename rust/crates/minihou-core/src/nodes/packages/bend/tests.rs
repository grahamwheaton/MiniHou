use super::*;
    #[test]
    fn quarter_circle_and_straight_extension(){let g=Arc::new(Geometry::points_only(vec![0.,0.,0.,0.,0.,1.,0.,0.,2.]).unwrap());let out=bend(&[vec![g],vec![]],&Params::resolve(NODE.params,None).unwrap()).unwrap();let r=2./std::f32::consts::PI;assert!((out.p()[3]-r).abs()<1e-6);assert!((out.p()[5]-r).abs()<1e-6);assert!((out.p()[6]-r-1.).abs()<1e-6);}
    #[test]
    fn rest_capture_adds_displacement_and_group_restricts(){let rest=Arc::new(Geometry::points_only(vec![0.,0.,1.,0.,0.,1.]).unwrap());let g=Arc::new(Geometry::points_only(vec![0.,2.,1.,0.,2.,1.]).unwrap());let p=Params::resolve(NODE.params,serde_json::json!({"group":"0"}).as_object()).unwrap();let out=bend(&[vec![g],vec![rest]],&p).unwrap();assert_eq!(out.p()[1],2.);assert_eq!(&out.p()[3..],&[0.,2.,1.]);}
