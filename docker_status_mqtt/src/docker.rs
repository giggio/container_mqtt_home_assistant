use bollard::{
    Docker,
    query_parameters::{ListContainersOptionsBuilder, ListImagesOptionsBuilder},
    secret::ContainerSummary,
};

pub async fn _list_containers() -> Result<Vec<ContainerSummary>, Box<dyn std::error::Error>> {
    let docker = Docker::connect_with_socket_defaults()?;
    let builder = ListContainersOptionsBuilder::new().all(true);
    let containers = docker.list_containers(Some(builder.build())).await?;
    Ok(containers)
}

pub async fn _list_images() -> Result<(), Box<dyn std::error::Error>> {
    let docker = Docker::connect_with_socket_defaults()?;
    let builder = ListImagesOptionsBuilder::new().all(true);
    let images = docker.list_images(Some(builder.build())).await?;
    for image in images {
        println!("-> {:?}", image);
    }
    Ok(())
}
