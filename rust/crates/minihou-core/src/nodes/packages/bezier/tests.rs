use super::*;

    #[test]
    fn bezier_vector_handles_and_optional_input(){
        let p=Params::resolve(NODE.params,serde_json::json!({"controls":"[{\"p\":[0,0,0],\"left\":[0,0,0],\"right\":[0,0,0],\"mode\":\"VECTOR\"},{\"p\":[3,0,0],\"left\":[0,0,0],\"right\":[0,0,0],\"mode\":\"VECTOR\"}]","resolution":3}).as_object()).unwrap();
        let g=bezier(&[vec![]],&p).unwrap();assert_eq!(g.p(),[0.,0.,0.,1.,0.,0.,2.,0.,0.,3.,0.,0.]);
        let g=bezier(&[vec![Arc::new(Geometry::points_only(vec![9.,9.,9.]).unwrap())]],&p).unwrap();assert_eq!(g.npoints,5);
    }
